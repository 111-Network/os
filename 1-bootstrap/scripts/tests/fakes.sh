# shellcheck shell=sh
# fakes — stand-ins shared by the hermetic gates in scripts/tests/ (sourced, never run). They let a
# gate drive scripts/cross-build.sh, scripts/check.sh and ../50-messaging/scripts/check.sh on a
# synthetic tree: no cargo, no cross compiler, no network (Wave 2 review B1 m1-m3).

# sysbin DIR TOOL... — DIR holds links to the named host tools and nothing else, so a gate's PATH
# lets no other host tool (a system pnpm, a real cargo, a cross compiler) leak in. A missing host
# tool fails the gate: it never passes by skipping.
sysbin() {
  _sb_dir=$1
  shift
  mkdir -p "$_sb_dir"
  for _sb_t in "$@"; do
    _sb_p=$(command -v "$_sb_t" 2>/dev/null) || _sb_p=
    case "$_sb_p" in
      /*) ln -sf "$_sb_p" "$_sb_dir/$_sb_t" ;;
      *)
        echo "FAIL: host tool $_sb_t is missing (this gate needs it)" >&2
        return 1
        ;;
    esac
  done
}

# stub NAME DIR [BODY] — an executable DIR/NAME running the sh BODY (default: exit 0).
stub() {
  mkdir -p "$2"
  printf '#!/bin/sh\n%s\n' "${3:-exit 0}" >"$2/$1"
  chmod 0755 "$2/$1"
}

# fake_toolchains HOME ARCH... — stub <arch>-linux-musl-gcc/-ar where scripts/musl-env.sh looks for
# the no-sudo cross toolchains (HOME/.local/toolchains/*/bin).
fake_toolchains() {
  _ft_home=$1
  shift
  for _ft_a in "$@"; do
    for _ft_t in gcc ar; do
      stub "$_ft_a-linux-musl-$_ft_t" "$_ft_home/.local/toolchains/$_ft_a-linux-musl-cross/bin"
    done
  done
}

# fake_readelf DIR — a `readelf` for fake binaries (fake_bin): -hW prints the FAKE-MACHINE marker
# as the Machine line, -lW prints an INTERP header when FAKE-INTERP is present and one LOAD header
# aligned 0x4000, -dW prints a NEEDED entry when FAKE-NEEDED is present.
fake_readelf() {
  # The $-expressions belong to the stub, not to this shell.
  # shellcheck disable=SC2016
  stub readelf "$1" 'f=$2
case "$1" in
  -hW) echo "  Machine:                           $(sed -n "s/.*FAKE-MACHINE=\([A-Za-z0-9-]*\).*/\1/p" "$f" | head -n 1)" ;;
  -lW)
    if grep -q FAKE-INTERP "$f"; then echo "  INTERP         0x0002a8 0x00000000000002a8 0x00000000000002a8 0x00001c 0x00001c R   0x1"; fi
    echo "  LOAD           0x000000 0x0000000000000000 0x0000000000000000 0x000100 0x000100 R E 0x4000" ;;
  -dW)
    if grep -q FAKE-NEEDED "$f"; then echo " 0x0000000000000001 (NEEDED)             Shared library: [libc.so]"; fi ;;
esac
exit 0'
}

# fake_bin FILE MACHINE TEXT — a fake binary for fake_readelf: a sh script that prints
# 2026.10.14 (for `--version`) and carries TEXT, the strings the gates' asserts look for.
fake_bin() {
  mkdir -p "$(dirname "$1")"
  printf '#!/bin/sh\n# FAKE-MACHINE=%s %s\necho 2026.10.14\n' "$2" "$3" >"$1"
  chmod 0755 "$1"
}

# What SQLCipher's and OpenSSL's C code put in a binary, and what only mesh111's own Rust code
# does (the pre-bounce assert accepted the latter: Wave 2 review B1 m1).
CODEC='cipher_plaintext_header_size OpenSSL 3.6.3 9 Jun 2026'
OWN_STRINGS='PRAGMA cipher_version SQLCipher is not linked (refusing a plaintext store) OpenSSL 3.6.3 9 Jun 2026'
export CODEC OWN_STRINGS

# fake_lock FILE [mesh111] — a Cargo.lock with openssl-src 300.6.1+3.6.3 (and mesh111 when asked).
fake_lock() {
  printf '[[package]]\nname = "openssl-src"\nversion = "300.6.1+3.6.3"\n\n' >"$1"
  if [ "${2:-}" = mesh111 ]; then
    printf '[[package]]\nname = "mesh111"\nversion = "0.1.0"\n\n' >>"$1"
  fi
}
