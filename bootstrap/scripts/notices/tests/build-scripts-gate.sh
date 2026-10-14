#!/bin/sh
# Hermetic gate for meshmud-build.sh and meshtastic-web-build.sh: their verify paths, run against
# synthetic tarballs (git archive of a scratch repo, hand-made release assets) with the pins
# rewritten in a COPY of each script. No network, no Go, no pnpm. Needs git, tar, gzip, sha256sum.
#
# usage: scripts/notices/tests/build-scripts-gate.sh
set -u

HERE=$(cd "$(dirname "$0")" && pwd)
SCRIPTS=$(dirname "$(dirname "$HERE")")
T=$(mktemp -d "${TMPDIR:-/tmp}/build-scripts-gate.XXXXXX")
trap 'rm -rf "$T"' EXIT
FAILS=0
pass() { printf '%s\n' "build-gate: ok: $*"; }
fail() { printf '%s\n' "build-gate: FAIL: $*" >&2; FAILS=$((FAILS + 1)); }

# expect_ok NAME CMD...    expect_out NAME PATTERN CMD... (succeeds and says PATTERN)    expect_fail NAME PATTERN CMD...
expect_ok() {
  name=$1; shift
  if out=$("$@" 2>&1); then pass "$name"; else fail "$name: exited non-zero: $out"; fi
}
expect_out() {
  name=$1; pat=$2; shift 2
  if ! out=$("$@" 2>&1); then fail "$name: exited non-zero: $out"; return; fi
  case "$out" in *"$pat"*) pass "$name" ;; *) fail "$name: wrong output (wanted '$pat'): $out" ;; esac
}
expect_fail() {
  name=$1; pat=$2; shift 2
  if out=$("$@" 2>&1); then fail "$name: expected a failure, got success"; return; fi
  case "$out" in *"$pat"*) pass "$name" ;; *) fail "$name: wrong message (wanted '$pat'): $out" ;; esac
}

# ---------------------------------------------------------------- meshmud-build.sh
M="$T/meshmud"
mkdir -p "$M/repo" "$M/scripts/notices"
(
  cd "$M/repo" || exit 1
  git init -q .
  printf 'module example.com/meshmud\n\ngo 1.24.4\n\nrequire github.com/samber/lo v1.53.0\n' > go.mod
  printf 'github.com/samber/lo v1.53.0 h1:AAAA=\ngithub.com/samber/lo v1.53.0/go.mod h1:BBBB=\n' > go.sum
  printf 'MIT License\n' > LICENSE
  git add -A
  git -c user.name=t -c user.email=t@example.invalid commit -q -m init
  git archive --format=tar.gz --prefix=meshmud-1.0.0/ -o "$M/meshmud.tar.gz" HEAD
)
COMMIT=$(git -C "$M/repo" rev-parse HEAD)
SHA=$(sha256sum "$M/meshmud.tar.gz" | awk '{print $1}')
cp "$M/repo/go.mod" "$M/scripts/notices/meshmud-v9.9.9.go.mod"
cp "$M/repo/go.sum" "$M/scripts/notices/meshmud-v9.9.9.go.sum"
sed -e 's/^MESHMUD_TAG=.*/MESHMUD_TAG="v9.9.9"/' \
    -e "s/^MESHMUD_COMMIT=.*/MESHMUD_COMMIT=\"$COMMIT\"/" \
    -e "s/^MESHMUD_TARBALL_SHA256=.*/MESHMUD_TARBALL_SHA256=\"$SHA\"/" \
    "$SCRIPTS/meshmud-build.sh" > "$M/scripts/meshmud-build.sh"

run_m() { sh "$M/scripts/meshmud-build.sh" "$@"; }
expect_ok   "meshmud: good tarball and snapshots verify" run_m --verify-only --tarball "$M/meshmud.tar.gz"

cp "$M/meshmud.tar.gz" "$M/tampered.tar.gz"
printf 'x' >> "$M/tampered.tar.gz"
expect_fail "meshmud: an altered tarball is refused" "tarball sha256 mismatch" run_m --verify-only --tarball "$M/tampered.tar.gz"

sed "s/^MESHMUD_COMMIT=.*/MESHMUD_COMMIT=\"0000000000000000000000000000000000000000\"/" "$M/scripts/meshmud-build.sh" > "$M/scripts/wrong-commit.sh"
expect_fail "meshmud: a wrong commit pin is refused" "tarball commit mismatch" sh "$M/scripts/wrong-commit.sh" --verify-only --tarball "$M/meshmud.tar.gz"

sed 's/^MESHMUD_TARBALL_SHA256=.*/MESHMUD_TARBALL_SHA256="TODO"/' "$M/scripts/meshmud-build.sh" > "$M/scripts/todo-pin.sh"
expect_fail "meshmud: a TODO pin is a hard stop" "is not set" sh "$M/scripts/todo-pin.sh" --verify-only --tarball "$M/meshmud.tar.gz"

printf '// indirect drift\n' >> "$M/scripts/notices/meshmud-v9.9.9.go.sum"
expect_fail "meshmud: go.sum drift against the notices snapshot is refused" "differs from scripts/notices" run_m --verify-only --tarball "$M/meshmud.tar.gz"
cp "$M/repo/go.sum" "$M/scripts/notices/meshmud-v9.9.9.go.sum"

# no Go toolchain on PATH: a build (not --verify-only) must stop with the CI-only message
BIN="$M/bin"
mkdir -p "$BIN"
for tool in sha256sum tar gzip sed cmp awk mktemp head tr find dirname cat rm mkdir cp sort wc curl sh; do
  p=$(command -v "$tool" 2>/dev/null || true)
  [ -n "$p" ] && ln -sf "$p" "$BIN/$tool"
done
expect_fail "meshmud: a build without Go says Go is CI-only" "CI-only" env PATH="$BIN" sh "$M/scripts/meshmud-build.sh" --tarball "$M/meshmud.tar.gz" --out "$M/out"

# ---------------------------------------------------------------- meshtastic-web-build.sh
W="$T/web"
mkdir -p "$W/scripts" "$W/inc" "$W/comp"
printf '<script src="/app.js"></script>\n' > "$W/inc/index.html"
printf 'fetch("/db.wasm")\n' > "$W/inc/app.js"
cp "$W/inc/index.html" "$W/inc/app.js" "$W/comp/"
printf 'WASM' > "$W/comp/db.wasm"
for d in inc comp; do
  (cd "$W/$d" && for f in *; do gzip -n -c "$f" > "$f.gz"; rm "$f"; done)
  (cd "$W/$d" && tar -cf "$W/$d.tar" .)
done
mkdir -p "$W/evil"
(cd "$W/evil" && printf 'x' > real && ln -s /etc/passwd link && tar -cf "$W/evil.tar" .)

pin_web() { # TARFILE OUTSCRIPT
  sha=$(sha256sum "$1" | awk '{print $1}')
  sed -e "s/^WEB_ASSET_SHA256=.*/WEB_ASSET_SHA256=\"$sha\"/" -e 's/^WEB_TREE_SHA256=.*/WEB_TREE_SHA256=""/' \
    "$SCRIPTS/meshtastic-web-build.sh" > "$2"
}
pin_web "$W/inc.tar" "$W/scripts/incomplete.sh"
pin_web "$W/comp.tar" "$W/scripts/complete.sh"
pin_web "$W/evil.tar" "$W/scripts/evil.sh"

expect_out  "web: an incomplete asset is not used (auto falls back to the tag)" "would build from the tag" sh "$W/scripts/incomplete.sh" --verify-only --asset "$W/inc.tar" --work "$W/w1"
expect_out  "web: ... and names the missing file" "/db.wasm" sh "$W/scripts/incomplete.sh" --verify-only --asset "$W/inc.tar" --work "$W/w1b"
expect_fail "web: --mode asset refuses an incomplete asset" "incomplete" sh "$W/scripts/incomplete.sh" --mode asset --asset "$W/inc.tar" --out "$W/o1" --work "$W/w2"
expect_out  "web: a complete asset is used by auto" "auto would use the release asset" sh "$W/scripts/complete.sh" --verify-only --asset "$W/comp.tar" --work "$W/w3"
expect_fail "web: links inside the asset are refused" "links or special files" sh "$W/scripts/evil.sh" --verify-only --asset "$W/evil.tar" --work "$W/w4"
expect_fail "web: an altered asset is refused" "asset sha256 mismatch" sh "$W/scripts/complete.sh" --verify-only --asset "$W/inc.tar" --work "$W/w5"

if [ "$FAILS" -ne 0 ]; then
  printf '%s\n' "build-gate: $FAILS failure(s)" >&2
  exit 1
fi
printf '%s\n' "build-gate: all ok"
