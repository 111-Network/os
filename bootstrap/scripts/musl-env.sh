# shellcheck shell=sh
# musl-env — the musl cross-toolchain environment and the static-link assertions shared by
# scripts/check.sh and scripts/cross-build.sh. Sourced (`. scripts/musl-env.sh`), never run.
#
# SQLCipher + a vendored OpenSSL (Layer 5 `mesh111`; os/messaging/README.md §10.1, §19 L1, §20 G2)
# are C code that cc-rs / openssl-src compile for each musl target:
#
# - PATH gains the no-sudo cross toolchains at ~/.local/toolchains/*/bin.
# - Linkers, CC and AR are pinned per target, so a missing cross compiler is a hard error, never a
#   silent fallback to the host cc. ../messaging/scripts/check.sh exports the same values: identical
#   env, so neither workspace rebuilds the C objects because the other one ran.
# - The escape hatches that would link a system SQLite or a system libcrypto instead of the bundled
#   SQLCipher and the vendored OpenSSL are unset: LIBSQLITE3_SYS_USE_PKG_CONFIG and its SQLCIPHER_*
#   companions (libsqlite3-sys), OPENSSL_NO_VENDOR / OPENSSL_DIR / OPENSSL_LIB_DIR /
#   OPENSSL_INCLUDE_DIR / OPENSSL_STATIC / OPENSSL_LIBS / OPENSSL_CONFIG_DIR, each also in its
#   <TARGET>_ form (openssl-sys reads both), and LIBSQLITE3_FLAGS (it can switch codec defines off).
# - static_elf / sqlcipher_linked / openssl_src_version are the assertions the gates run.

for tc in "$HOME"/.local/toolchains/*/bin; do
  if [ -d "$tc" ]; then
    PATH="$tc:$PATH"
  fi
done
export PATH

export CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_LINKER=x86_64-linux-musl-gcc
export CARGO_TARGET_AARCH64_UNKNOWN_LINUX_MUSL_LINKER=aarch64-linux-musl-gcc
export CC_x86_64_unknown_linux_musl=x86_64-linux-musl-gcc
export AR_x86_64_unknown_linux_musl=x86_64-linux-musl-ar
export CC_aarch64_unknown_linux_musl=aarch64-linux-musl-gcc
export AR_aarch64_unknown_linux_musl=aarch64-linux-musl-ar

for v in LIBSQLITE3_SYS_USE_PKG_CONFIG LIBSQLITE3_FLAGS SQLCIPHER_LIB_DIR SQLCIPHER_INCLUDE_DIR \
  SQLCIPHER_STATIC SQLITE3_LIB_DIR SQLITE3_INCLUDE_DIR SQLITE3_STATIC; do
  unset "$v"
done
for p in "" X86_64_UNKNOWN_LINUX_MUSL_ AARCH64_UNKNOWN_LINUX_MUSL_; do
  for v in OPENSSL_NO_VENDOR OPENSSL_DIR OPENSSL_LIB_DIR OPENSSL_INCLUDE_DIR OPENSSL_STATIC \
    OPENSSL_LIBS OPENSSL_CONFIG_DIR; do
    unset "$p$v"
  done
done

# musl_toolchains_present — both cross compilers (and their ar) are on PATH.
musl_toolchains_present() {
  for t in x86_64-linux-musl-gcc x86_64-linux-musl-ar aarch64-linux-musl-gcc aarch64-linux-musl-ar; do
    command -v "$t" >/dev/null 2>&1 || return 1
  done
}

# openssl_src_version LOCKFILE — the OpenSSL release openssl-src vendors: the `+3.6.3` suffix of
# the crate version in LOCKFILE. Empty when the lock has no openssl-src.
openssl_src_version() {
  sed -n '/^name = "openssl-src"$/{n;s/^version = "[^"+]*+\([^"]*\)"$/\1/p;}' "$1"
}

# static_elf BIN MACHINE — fail unless BIN is a static ELF (no PT_INTERP program header, no
# DT_NEEDED entry) for MACHINE, a substring of readelf's "Machine:" line (X86-64, AArch64).
static_elf() {
  if readelf -lW "$1" | grep -q INTERP || readelf -dW "$1" 2>/dev/null | grep -q NEEDED; then
    echo "FAIL: $1 is dynamically linked" >&2
    return 1
  fi
  if ! readelf -hW "$1" | grep -q "Machine:.*$2"; then
    echo "FAIL: $1 is not an $2 binary" >&2
    return 1
  fi
}

# sqlcipher_linked BIN OPENSSL_VERSION — fail unless BIN carries SQLCipher's codec and the
# vendored OpenSSL. Both strings come only from their C sources, never from ours (Wave 2 review
# B1 m1: mesh111's own "PRAGMA cipher_version" and "SQLCipher is not linked …" strings satisfied
# the old assert even in a plain-SQLite build):
#   cipher_plaintext_header_size  a SQLCipher-only pragma name (absent from plain SQLite);
#   "OpenSSL <ver> "              OpenSSL's version text (absent with no or a dynamic libcrypto).
sqlcipher_linked() {
  if ! LC_ALL=C grep -aq 'cipher_plaintext_header_size' "$1"; then
    echo "FAIL: $1 carries no SQLCipher codec (cipher_plaintext_header_size)" >&2
    return 1
  fi
  if ! LC_ALL=C grep -aqF "OpenSSL $2 " "$1"; then
    echo "FAIL: $1 carries no statically linked OpenSSL $2" >&2
    return 1
  fi
}
