#!/bin/sh
# cross-build — musl-static `111` for x86_64 + aarch64 (spec §Artifacts).
# Toolchains: musl.cc cross compilers at ~/.local/toolchains (no sudo).
# aarch64 links with -z max-page-size=16384 (16KB-page Android kernels) and the
# script asserts p_align=0x4000 (spec CI assert).
#
# SQLCipher (Layer 5, os/50-messaging/README.md §10.1, §19 L1): once node111 depends on `mesh111`, the
# release links SQLCipher 4.x + a vendored OpenSSL 3 (rusqlite `bundled-sqlcipher-vendored-openssl`,
# built from crate sources by cc-rs / openssl-src: no system libcrypto, nothing downloaded). The env
# comes from scripts/musl-env.sh (shared with scripts/check.sh): CC/AR/linkers pinned per target, so
# a missing cross compiler is a hard error, never a silent host-gcc fallback, and the escape hatches
# that would link a system SQLite or libcrypto (LIBSQLITE3_SYS_USE_PKG_CONFIG, OPENSSL_NO_VENDOR,
# OPENSSL_DIR, … and their per-target forms) unset. The asserts then check both binaries are static
# ELFs (no INTERP, no NEEDED) that carry strings only SQLCipher's and OpenSSL's C code has
# (`cipher_plaintext_header_size`, `OpenSSL <ver> ` with <ver> from Cargo.lock's openssl-src);
# without the dependency they say so. The size of each binary is printed (Layer 5 binary-growth
# record: 10-bootstrap/TASKS.md, B1).
#
# CARGO_TARGET_DIR is honoured (parallel builders keep private target dirs).
#
# usage: scripts/cross-build.sh [--allow-no-ui] CALVER [OUTDIR]
# The release embeds 40-interface/ui/dist. With pnpm the UI is always rebuilt;
# without pnpm the build FAILS unless dist/ exists and is newer than every UI
# build input (src/, public/, index.html, package.json, pnpm-lock.yaml,
# vite.config.ts). --allow-no-ui skips that gate
# (test builds only: such a binary serves the "dashboard UI not built" notice).
set -eu

cd "$(dirname "$0")/.."
ALLOW_NO_UI=0
if [ "${1:-}" = "--allow-no-ui" ]; then
  ALLOW_NO_UI=1
  shift
fi
VER=${1:?usage: scripts/cross-build.sh [--allow-no-ui] CALVER [OUTDIR]}
case "$VER" in
  *[!0-9.]*) echo "calver expected (YYYY.MM.DD[.N]), got $VER" >&2; exit 2 ;;
esac

export PATH="$HOME/.cargo/bin:$PATH"
# musl toolchains + pinned CC/AR/linkers + escape hatches unset + the static_elf / sqlcipher_linked /
# openssl_src_version assertions (the same env scripts/check.sh builds G2 with).
# shellcheck source=scripts/musl-env.sh
. scripts/musl-env.sh
musl_toolchains_present || {
  echo "FAIL: musl cross toolchains absent (~/.local/toolchains: x86_64- and aarch64-linux-musl-gcc/-ar)" >&2
  exit 1
}
export NODE111_BUILD_VERSION="$VER"
TARGET_DIR=${CARGO_TARGET_DIR:-target}

OUT=${2:-dist/releases/$VER}
mkdir -p "$OUT"

# Layer 4 UI: release builds embed 40-interface/ui/dist at COMPILE time (rust-embed),
# so build it before cargo. A release must never ship a stale or missing UI.
UI_DIR=../40-interface/ui
ui_stale() { # prints the first UI source newer than dist/index.html
  find "$UI_DIR/src" "$UI_DIR/public" "$UI_DIR/index.html" "$UI_DIR/package.json" \
    "$UI_DIR/pnpm-lock.yaml" "$UI_DIR/vite.config.ts" -newer "$UI_DIR/dist/index.html" -print 2>/dev/null | head -n 1
}
if command -v pnpm >/dev/null 2>&1; then
  echo "== build dashboard UI (pnpm → 40-interface/ui/dist, embedded below) =="
  if [ ! -f "$UI_DIR/node_modules/.modules.yaml" ]; then
    pnpm --dir "$UI_DIR" install --frozen-lockfile
  fi
  pnpm --dir "$UI_DIR" build
  [ -f "$UI_DIR/dist/index.html" ] || { echo "FAIL: UI build produced no dist/index.html" >&2; exit 1; }
elif [ "$ALLOW_NO_UI" = 1 ]; then
  echo "!! pnpm not found, --allow-no-ui given — embedding 40-interface/ui/dist as-is (or none: / serves the 'dashboard UI not built' notice). NOT a release build."
elif [ ! -f "$UI_DIR/dist/index.html" ]; then
  echo "FAIL: pnpm not found and 40-interface/ui/dist is empty — this release would ship without the dashboard UI. Install pnpm, or pass --allow-no-ui for a test build." >&2
  exit 1
elif [ -n "$(ui_stale)" ]; then
  echo "FAIL: pnpm not found and 40-interface/ui/dist is stale ($(ui_stale) is newer) — install pnpm to rebuild it, or pass --allow-no-ui for a test build." >&2
  exit 1
else
  echo "!! pnpm not found — 40-interface/ui/dist is newer than every UI source; embedding it as-is"
fi

echo "== build x86_64-unknown-linux-musl (release) =="
cargo build --release --target x86_64-unknown-linux-musl
cp "$TARGET_DIR/x86_64-unknown-linux-musl/release/111" "$OUT/111-x86_64-unknown-linux-musl"

echo "== build aarch64-unknown-linux-musl (release, 16KB pages) =="
cargo build --release --target aarch64-unknown-linux-musl
cp "$TARGET_DIR/aarch64-unknown-linux-musl/release/111" "$OUT/111-aarch64-unknown-linux-musl"

echo "== asserts =="
static_elf "$OUT/111-x86_64-unknown-linux-musl" X86-64
echo "ok: x86_64 static (no INTERP, no NEEDED)"
static_elf "$OUT/111-aarch64-unknown-linux-musl" AArch64
echo "ok: aarch64 static AArch64 (no INTERP, no NEEDED)"

ALIGN=$(readelf -lW "$OUT/111-aarch64-unknown-linux-musl" | awk '$1=="LOAD"{print $NF; exit}')
echo "aarch64 LOAD align: $ALIGN"
[ "$ALIGN" = "0x4000" ] || { echo "FAIL: expected p_align 0x4000 (16KB pages)" >&2; exit 1; }

# SQLCipher rides in only through `mesh111` (Layer 5, F2's path dependency). When the lock has it, both
# binaries must carry SQLCipher's codec and the vendored OpenSSL — strings only their C code carries
# (scripts/musl-env.sh `sqlcipher_linked`), so a build that fell back to plain SQLite or a system
# libcrypto, or dropped the store, fails here.
if grep -q '^name = "mesh111"$' Cargo.lock; then
  OSSL_VER=$(openssl_src_version Cargo.lock)
  [ -n "$OSSL_VER" ] || { echo "FAIL: Cargo.lock has mesh111 but no openssl-src (+<OpenSSL version>)" >&2; exit 1; }
  for t in x86_64 aarch64; do
    sqlcipher_linked "$OUT/111-$t-unknown-linux-musl" "$OSSL_VER"
    echo "ok: $t links SQLCipher (cipher_plaintext_header_size) + OpenSSL $OSSL_VER, statically"
  done
else
  echo "note: node111 does not depend on mesh111 yet (Layer 5 task F2) — SQLCipher assert not applicable"
fi

echo "== sizes (bytes; Layer 5 growth is recorded in 10-bootstrap/TASKS.md, B1) =="
for t in x86_64 aarch64; do
  printf '%s %s\n' "$t" "$(wc -c <"$OUT/111-$t-unknown-linux-musl" | tr -d ' ')"
done

GOT=$("$OUT/111-x86_64-unknown-linux-musl" --version)
[ "$GOT" = "$VER" ] || { echo "FAIL: --version printed $GOT, want $VER" >&2; exit 1; }
echo "ok: --version = $VER (x86_64 run here; aarch64 run is gated to V13/physical)"

echo "cross-build OK → $OUT"
