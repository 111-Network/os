#!/bin/sh
# cross-build — musl-static `111` for x86_64 + aarch64 (spec §Artifacts).
# Toolchains: musl.cc cross compilers at ~/.local/toolchains (no sudo).
# aarch64 links with -z max-page-size=16384 (16KB-page Android kernels) and the
# script asserts p_align=0x4000 (spec CI assert).
#
# usage: scripts/cross-build.sh [--allow-no-ui] CALVER [OUTDIR]
# The release embeds interface/ui/dist. With pnpm the UI is always rebuilt;
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
for tc in "$HOME"/.local/toolchains/*/bin; do
  [ -d "$tc" ] && PATH="$tc:$PATH"
done
export PATH

export CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_LINKER=x86_64-linux-musl-gcc
export CARGO_TARGET_AARCH64_UNKNOWN_LINUX_MUSL_LINKER=aarch64-linux-musl-gcc
export NODE111_BUILD_VERSION="$VER"

OUT=${2:-dist/releases/$VER}
mkdir -p "$OUT"

# Layer 4 UI: release builds embed interface/ui/dist at COMPILE time (rust-embed),
# so build it before cargo. A release must never ship a stale or missing UI.
UI_DIR=../interface/ui
ui_stale() { # prints the first UI source newer than dist/index.html
  find "$UI_DIR/src" "$UI_DIR/public" "$UI_DIR/index.html" "$UI_DIR/package.json" \
    "$UI_DIR/pnpm-lock.yaml" "$UI_DIR/vite.config.ts" -newer "$UI_DIR/dist/index.html" -print 2>/dev/null | head -n 1
}
if command -v pnpm >/dev/null 2>&1; then
  echo "== build dashboard UI (pnpm → interface/ui/dist, embedded below) =="
  if [ ! -f "$UI_DIR/node_modules/.modules.yaml" ]; then
    pnpm --dir "$UI_DIR" install --frozen-lockfile
  fi
  pnpm --dir "$UI_DIR" build
  [ -f "$UI_DIR/dist/index.html" ] || { echo "FAIL: UI build produced no dist/index.html" >&2; exit 1; }
elif [ "$ALLOW_NO_UI" = 1 ]; then
  echo "!! pnpm not found, --allow-no-ui given — embedding interface/ui/dist as-is (or none: / serves the 'dashboard UI not built' notice). NOT a release build."
elif [ ! -f "$UI_DIR/dist/index.html" ]; then
  echo "FAIL: pnpm not found and interface/ui/dist is empty — this release would ship without the dashboard UI. Install pnpm, or pass --allow-no-ui for a test build." >&2
  exit 1
elif [ -n "$(ui_stale)" ]; then
  echo "FAIL: pnpm not found and interface/ui/dist is stale ($(ui_stale) is newer) — install pnpm to rebuild it, or pass --allow-no-ui for a test build." >&2
  exit 1
else
  echo "!! pnpm not found — interface/ui/dist is newer than every UI source; embedding it as-is"
fi

echo "== build x86_64-unknown-linux-musl (release) =="
cargo build --release --target x86_64-unknown-linux-musl
cp target/x86_64-unknown-linux-musl/release/111 "$OUT/111-x86_64-unknown-linux-musl"

echo "== build aarch64-unknown-linux-musl (release, 16KB pages) =="
cargo build --release --target aarch64-unknown-linux-musl
cp target/aarch64-unknown-linux-musl/release/111 "$OUT/111-aarch64-unknown-linux-musl"

echo "== asserts =="
if file "$OUT/111-x86_64-unknown-linux-musl" | grep -q "static-pie linked\|statically linked"; then
  echo "ok: x86_64 static"
else
  file "$OUT/111-x86_64-unknown-linux-musl"
  echo "FAIL: x86_64 not static" >&2
  exit 1
fi
if file "$OUT/111-aarch64-unknown-linux-musl" | grep -q "static-pie linked\|statically linked"; then
  echo "ok: aarch64 static"
else
  file "$OUT/111-aarch64-unknown-linux-musl"
  echo "FAIL: aarch64 not static" >&2
  exit 1
fi

ALIGN=$(readelf -lW "$OUT/111-aarch64-unknown-linux-musl" | awk '$1=="LOAD"{print $NF; exit}')
echo "aarch64 LOAD align: $ALIGN"
[ "$ALIGN" = "0x4000" ] || { echo "FAIL: expected p_align 0x4000 (16KB pages)" >&2; exit 1; }

GOT=$("$OUT/111-x86_64-unknown-linux-musl" --version)
[ "$GOT" = "$VER" ] || { echo "FAIL: --version printed $GOT, want $VER" >&2; exit 1; }
echo "ok: --version = $VER (x86_64 run here; aarch64 run is gated to V13/physical)"

echo "cross-build OK → $OUT"
