#!/bin/sh
# musl-env-test — self-test of scripts/musl-env.sh, the environment and assertions the G2 stages of
# scripts/check.sh and the release asserts of scripts/cross-build.sh rely on (Wave 2 review B1 m1-m3).
# Hermetic: synthetic files only, no cargo, no network. Run from os/bootstrap (scripts/check.sh does).
set -eu
cd "$(dirname "$0")/.."

FAILS=0
fail() {
  echo "FAIL: $1" >&2
  FAILS=$((FAILS + 1))
}
T=$(mktemp -d)
trap 'rm -rf "$T"' EXIT

# 1. The escape hatches are unset, plain and per-target, and CC/AR/linkers are pinned.
LIBSQLITE3_SYS_USE_PKG_CONFIG=1 OPENSSL_NO_VENDOR=1 OPENSSL_DIR=/usr \
  X86_64_UNKNOWN_LINUX_MUSL_OPENSSL_DIR=/usr AARCH64_UNKNOWN_LINUX_MUSL_OPENSSL_NO_VENDOR=1 \
  OPENSSL_LIB_DIR=/usr/lib SQLCIPHER_LIB_DIR=/usr/lib LIBSQLITE3_FLAGS=-USQLITE_HAS_CODEC \
  sh -c '. scripts/musl-env.sh; env' >"$T/env"
for v in LIBSQLITE3_SYS_USE_PKG_CONFIG OPENSSL_NO_VENDOR OPENSSL_DIR X86_64_UNKNOWN_LINUX_MUSL_OPENSSL_DIR \
  AARCH64_UNKNOWN_LINUX_MUSL_OPENSSL_NO_VENDOR OPENSSL_LIB_DIR SQLCIPHER_LIB_DIR LIBSQLITE3_FLAGS; do
  if grep -q "^$v=" "$T/env"; then fail "musl-env.sh leaves $v set"; fi
done
for kv in CC_x86_64_unknown_linux_musl=x86_64-linux-musl-gcc AR_x86_64_unknown_linux_musl=x86_64-linux-musl-ar \
  CC_aarch64_unknown_linux_musl=aarch64-linux-musl-gcc AR_aarch64_unknown_linux_musl=aarch64-linux-musl-ar \
  CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_LINKER=x86_64-linux-musl-gcc \
  CARGO_TARGET_AARCH64_UNKNOWN_LINUX_MUSL_LINKER=aarch64-linux-musl-gcc; do
  grep -qx "$kv" "$T/env" || fail "musl-env.sh does not export $kv"
  # ../messaging/scripts/check.sh must export the very same values (no rebuild ping-pong).
  grep -qx "export $kv" ../messaging/scripts/check.sh || fail "../messaging/scripts/check.sh does not export $kv"
done

# shellcheck source=scripts/musl-env.sh
. scripts/musl-env.sh

# 2. openssl_src_version reads the `+<OpenSSL version>` suffix from a lockfile.
printf '[[package]]\nname = "openssl-src"\nversion = "300.6.1+3.6.3"\nsource = "x"\n' >"$T/lock"
[ "$(openssl_src_version "$T/lock")" = 3.6.3 ] || fail "openssl_src_version did not read 3.6.3"
printf '[[package]]\nname = "openssl-sys"\nversion = "0.9.117"\n' >"$T/lock"
[ -z "$(openssl_src_version "$T/lock")" ] || fail "openssl_src_version invented a version"

# 3. sqlcipher_linked: mesh111's own strings (the old assert's false positive) are not enough.
printf 'x PRAGMA cipher_version x SQLCipher is not linked (refusing a plaintext store) x' >"$T/own-strings"
if sqlcipher_linked "$T/own-strings" 3.6.3 2>/dev/null; then
  fail "sqlcipher_linked accepted a binary carrying only mesh111's own strings"
fi
printf 'x PRAGMA cipher_version x SQLCipher is not linked x OpenSSL 3.6.3 9 Jun 2026 x' >"$T/own-strings-openssl"
if sqlcipher_linked "$T/own-strings-openssl" 3.6.3 2>/dev/null; then
  fail "sqlcipher_linked accepted mesh111's own SQLCipher strings as the codec"
fi
printf 'x cipher_plaintext_header_size x' >"$T/no-openssl"
if sqlcipher_linked "$T/no-openssl" 3.6.3 2>/dev/null; then
  fail "sqlcipher_linked accepted a binary without OpenSSL"
fi
printf 'x cipher_plaintext_header_size x OpenSSL 3.6.30 1 Jan 2027 x' >"$T/other-openssl"
if sqlcipher_linked "$T/other-openssl" 3.6.3 2>/dev/null; then
  fail "sqlcipher_linked accepted another OpenSSL version"
fi
printf 'x OpenSSL 3.6.3 9 Jun 2026 x' >"$T/no-sqlcipher"
if sqlcipher_linked "$T/no-sqlcipher" 3.6.3 2>/dev/null; then
  fail "sqlcipher_linked accepted a binary without SQLCipher"
fi
printf '\001cipher_plaintext_header_size\000OpenSSL 3.6.3 9 Jun 2026\000' >"$T/both"
sqlcipher_linked "$T/both" 3.6.3 || fail "sqlcipher_linked refused a binary carrying both"

# 4. static_elf refuses a dynamically linked ELF and a non-ELF.
if command -v readelf >/dev/null 2>&1; then
  for dyn in /bin/sh /usr/bin/env; do
    if [ -f "$dyn" ] && readelf -lW "$dyn" 2>/dev/null | grep -q INTERP; then
      if static_elf "$dyn" X86-64 2>/dev/null || static_elf "$dyn" AArch64 2>/dev/null; then
        fail "static_elf accepted the dynamic $dyn"
      fi
      break
    fi
  done
  if static_elf "$T/both" X86-64 2>/dev/null; then fail "static_elf accepted a non-ELF file"; fi
else
  fail "readelf is required (binutils): static_elf cannot be tested"
fi

[ "$FAILS" = 0 ] || exit 1
echo "ok: musl-env.sh — escape hatches unset, CC/AR/linkers pinned (same as messaging), openssl_src_version, sqlcipher_linked, static_elf"
