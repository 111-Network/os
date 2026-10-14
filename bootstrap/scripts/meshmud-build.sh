#!/bin/sh
# meshmud-build: build meshmud (the Meshtastic multiplexer, adopted) from a PINNED upstream
# source tarball into static linux/amd64 + linux/arm64 binaries (spec messaging/README.md
# section 2 and section 14, D19 g / D20 b).
#
# CI ONLY. Go is a CI-only toolchain: no Go is installed on a dev box or a node, nothing here runs
# on a node, and nothing downloads at runtime. The release workflow runs this once per release;
# the binaries go into the signed channel manifest. meshmud's source is never copied into os/: the
# only thing committed is the pin below plus the data files in scripts/notices/ (go.mod / go.sum
# snapshots and the hand-maintained license list).
#
# usage: scripts/meshmud-build.sh [--verify-only] [--tarball FILE] [--out DIR] [--work DIR]
#   --verify-only   fetch (or take --tarball), check the tarball sha256, the tag's commit id and the
#                   go.mod / go.sum snapshots, then stop. Needs no Go toolchain.
#   --tarball FILE  use a local copy of the pinned tarball instead of downloading it
#   --out DIR       output directory (default: dist/meshmud/<tag>)
#   --work DIR      scratch directory (default: a fresh mktemp -d, removed on exit)
#
# Output (all under --out):
#   meshmud-<tag>-linux-amd64, meshmud-<tag>-linux-arm64   static binaries (CGO off, -trimpath)
#   meshmud-<tag>.sha256        sha256sum of both binaries
#   meshmud-<tag>.buildinfo     `go version -m` of both (module list + toolchain)
#   meshmud-<tag>.verdict       whether the binary links Meshtastic's GPL Go protobufs, and the
#                               license it must be conveyed under ("verify at build", D20 b)
#   meshmud-<tag>.SOURCE        tag, commit, tarball sha256 (the GPL source offer names these)
#   LICENSE.meshmud             upstream MIT license, ships beside the binaries
#
# Pins: ONE assignment per line, double-quoted. gen-notices.py reads these exact lines.
MESHMUD_REPO="https://github.com/Rikonardo/meshmud"
MESHMUD_TAG="v1.0.0"
MESHMUD_COMMIT="6d08378cf09c90e867babd4184546cbaaf11f679"
MESHMUD_TARBALL_SHA256="bb05a46cab567899c6cf71a03cdd61438b8bec52f761db64fd8cea96a9b47c2d"
MESHMUD_GO_MIN="1.24.4"

set -eu

HERE=$(cd "$(dirname "$0")" && pwd)
NOTICES="$HERE/notices"
TARBALL_URL="$MESHMUD_REPO/archive/refs/tags/$MESHMUD_TAG.tar.gz"
VERIFY_ONLY=0
TARBALL=""
OUT=""
WORK=""

die() { printf '%s\n' "meshmud-build: FAIL: $*" >&2; exit 1; }
say() { printf '%s\n' "== $*"; }

while [ $# -gt 0 ]; do
  case "$1" in
    --verify-only) VERIFY_ONLY=1 ;;
    --tarball) shift; [ $# -gt 0 ] || die "--tarball needs a file"; TARBALL=$1 ;;
    --out) shift; [ $# -gt 0 ] || die "--out needs a directory"; OUT=$1 ;;
    --work) shift; [ $# -gt 0 ] || die "--work needs a directory"; WORK=$1 ;;
    -h|--help) sed -n '2,30p' "$0"; exit 0 ;;
    *) die "unknown argument: $1" ;;
  esac
  shift
done

# A pin that is empty or still a TODO is a hard stop, never a silent skip.
need_pin() { # NAME VALUE
  case "$2" in
    ""|*TODO*) die "pin $1 is not set (fetch the tarball once, record its sha256 here, review the diff)" ;;
  esac
}
need_pin MESHMUD_TAG "$MESHMUD_TAG"
need_pin MESHMUD_COMMIT "$MESHMUD_COMMIT"
need_pin MESHMUD_TARBALL_SHA256 "$MESHMUD_TARBALL_SHA256"

for tool in sha256sum tar gzip sed cmp awk mktemp head tr; do
  command -v "$tool" >/dev/null 2>&1 || die "missing tool: $tool"
done

[ -n "$OUT" ] || OUT="dist/meshmud/$MESHMUD_TAG"
if [ -z "$WORK" ]; then
  WORK=$(mktemp -d "${TMPDIR:-/tmp}/meshmud-build.XXXXXX")
  trap 'rm -rf "$WORK"' EXIT
else
  mkdir -p "$WORK"
fi

# --- 1. the pinned tarball ------------------------------------------------------------------
if [ -z "$TARBALL" ]; then
  command -v curl >/dev/null 2>&1 || die "missing tool: curl (or pass --tarball FILE)"
  TARBALL="$WORK/meshmud-$MESHMUD_TAG.tar.gz"
  say "fetch $TARBALL_URL"
  curl --fail --silent --show-error --location --proto '=https' --tlsv1.2 --max-time 180 \
    --output "$TARBALL" "$TARBALL_URL" || die "download failed"
fi
[ -f "$TARBALL" ] || die "tarball not found: $TARBALL"

say "verify tarball sha256"
GOT=$(sha256sum "$TARBALL" | awk '{print $1}')
[ "$GOT" = "$MESHMUD_TARBALL_SHA256" ] ||
  die "tarball sha256 mismatch: pinned $MESHMUD_TARBALL_SHA256, got $GOT (GitHub regenerates archives rarely; if the bytes changed, re-pin only after checking the commit below)"

# git archive stores the commit id of the tag in the tar's pax global header; it survives any
# re-compression, so it is the second, byte-format-independent pin.
say "verify tag commit"
COMMIT=$(gzip -dc "$TARBALL" | head -c 1024 | tr -d '\000' | sed -n 's/.*comment=\([0-9a-f]\{40\}\).*/\1/p' | head -n 1)
[ "$COMMIT" = "$MESHMUD_COMMIT" ] || die "tarball commit mismatch: pinned $MESHMUD_COMMIT, archive says '${COMMIT:-none}'"

# --- 2. unpack safely -----------------------------------------------------------------------
say "unpack"
BAD=$(tar -tzf "$TARBALL" | awk '/^\// || /(^|\/)\.\.(\/|$)/ {print; exit}')
[ -z "$BAD" ] || die "unsafe path in tarball: $BAD"
SRC="$WORK/src"
rm -rf "$SRC"
mkdir -p "$SRC"
tar -xzf "$TARBALL" -C "$SRC" --no-same-owner --no-same-permissions
TOP=$(find "$SRC" -mindepth 1 -maxdepth 1 -type d | head -n 1)
if [ -z "$TOP" ] || [ ! -f "$TOP/go.mod" ] || [ ! -f "$TOP/go.sum" ]; then
  die "no go.mod / go.sum in the tarball"
fi
if [ -L "$TOP/go.mod" ] || [ -L "$TOP/go.sum" ]; then
  die "go.mod / go.sum are symlinks"
fi

# --- 3. the closure snapshots must match what the notices were written from ------------------
say "compare go.mod / go.sum with scripts/notices snapshots"
for f in go.mod go.sum; do
  cmp -s "$TOP/$f" "$NOTICES/meshmud-$MESHMUD_TAG.$f" ||
    die "$f differs from scripts/notices/meshmud-$MESHMUD_TAG.$f; update the snapshot, scripts/notices/go-licenses.txt, then run gen-notices.py"
done

if [ "$VERIFY_ONLY" = 1 ]; then
  say "verify-only: pins and snapshots are consistent ($MESHMUD_TAG, $MESHMUD_COMMIT)"
  exit 0
fi

# --- 4. toolchain (CI only) ------------------------------------------------------------------
command -v go >/dev/null 2>&1 || die "go not found. Go is a CI-only toolchain here: run this script in the release workflow (go >= $MESHMUD_GO_MIN), or use --verify-only"
GOVER=$(go env GOVERSION | sed 's/^go//')
OLDEST=$(printf '%s\n%s\n' "$MESHMUD_GO_MIN" "$GOVER" | sort -V | head -n 1)
[ "$OLDEST" = "$MESHMUD_GO_MIN" ] || die "go $GOVER is older than the required $MESHMUD_GO_MIN"
command -v readelf >/dev/null 2>&1 || die "missing tool: readelf"

mkdir -p "$OUT"
OUT=$(cd "$OUT" && pwd)

# --- 5. build: static, CGO off, reproducible flags -------------------------------------------
build_one() {
  arch=$1
  bin="$OUT/meshmud-$MESHMUD_TAG-linux-$arch"
  say "build linux/$arch (CGO_ENABLED=0)"
  (
    cd "$TOP"
    CGO_ENABLED=0 GOOS=linux GOARCH="$arch" GOFLAGS=-mod=readonly GOTOOLCHAIN=local \
      go build -trimpath -buildvcs=false -ldflags="-s -w -buildid=" -o "$bin" ./cmd
  ) || die "go build failed for $arch"
  # static: no program interpreter, no dynamic section
  if readelf -lW "$bin" | grep -q 'INTERP'; then die "$arch binary is dynamically linked"; fi
  if readelf -dW "$bin" 2>/dev/null | grep -q 'NEEDED'; then die "$arch binary has DT_NEEDED entries"; fi
  align=$(readelf -lW "$bin" | awk '$1=="LOAD"{print $NF; exit}')
  # 16 KiB LOAD alignment matters on Android (arm64) kernels only; Go linux/amd64 links at 0x1000 and that is fine there.
  if [ "$arch" = arm64 ]; then
    [ "$((align))" -ge 16384 ] || die "$arch LOAD align $align < 0x4000 (16 KiB pages on Android kernels)"
  fi
  say "ok: $arch static, LOAD align $align"
}
build_one amd64
build_one arm64

# --- 6. the Go closure must be exactly what the license list covers --------------------------
say "check the built closure against scripts/notices/go-licenses.txt"
LIST="$WORK/licensed.txt"
awk -F' *[|] *' '!/^#/ && NF >= 3 {print $1 " " $2}' "$NOTICES/go-licenses.txt" | sort -u > "$LIST"
: > "$OUT/meshmud-$MESHMUD_TAG.buildinfo"
for arch in amd64 arm64; do
  bin="$OUT/meshmud-$MESHMUD_TAG-linux-$arch"
  go version -m "$bin" >> "$OUT/meshmud-$MESHMUD_TAG.buildinfo"
  go version -m "$bin" | awk '$1=="dep" {print $2 " " $3}' | sort -u > "$WORK/deps-$arch.txt"
  MISSING=$(awk 'NR==FNR {have[$0]=1; next} !($0 in have) {print}' "$LIST" "$WORK/deps-$arch.txt")
  [ -z "$MISSING" ] || die "modules linked into the $arch binary but missing from go-licenses.txt: $MISSING"
done

# --- 7. verdict: does the binary link Meshtastic's GPL Go protobufs? (D20 b, "verify at build") ----
# Module names alone are not enough: also scan the pinned source tree for generated protobuf code or Meshtastic
# imports (a vendored .pb.go would not show up as a module).
SRC_HITS=$(find "$SRC" -type f \( -name '*.pb.go' -o -name '*.go' \) -print0 2>/dev/null | xargs -0 grep -lE 'meshtastic/go|github.com/meshtastic|google.golang.org/protobuf' 2>/dev/null | head -5 || true)
if cat "$WORK/deps-amd64.txt" "$WORK/deps-arm64.txt" | grep -qi 'meshtastic' || [ -n "$SRC_HITS" ] || find "$SRC" -name '*.pb.go' -print -quit 2>/dev/null | grep -q .; then
  LINKS=yes
  CONVEY="GPL-3.0-only"
  say "!! Meshtastic protobuf code found (module, .pb.go or import; hits: ${SRC_HITS:-module list}): the binary must be conveyed under GPL-3.0 with the source offer in THIRD-PARTY-NOTICES.md"
else
  LINKS=no
  CONVEY="MIT (own code) plus the permissive module licenses in THIRD-PARTY-NOTICES.md"
  say "no Meshtastic protobuf module in the closure (meshmud $MESHMUD_TAG decodes the few fields it needs with its own wire parser)"
fi
{
  echo "tag=$MESHMUD_TAG"
  echo "links_meshtastic_protobufs=$LINKS"
  echo "convey_as=$CONVEY"
  echo "note=a courtesy source pointer is in THIRD-PARTY-NOTICES.md either way; a GPL source offer applies only when links_meshtastic_protobufs=yes"
} > "$OUT/meshmud-$MESHMUD_TAG.verdict"

# --- 8. provenance and notices that travel with the binaries ----------------------------------
{
  echo "project=$MESHMUD_REPO"
  echo "tag=$MESHMUD_TAG"
  echo "commit=$MESHMUD_COMMIT"
  echo "tarball=$TARBALL_URL"
  echo "tarball_sha256=$MESHMUD_TARBALL_SHA256"
  echo "go=$GOVER"
} > "$OUT/meshmud-$MESHMUD_TAG.SOURCE"
cp "$TOP/LICENSE" "$OUT/LICENSE.meshmud"
(
  cd "$OUT"
  sha256sum "meshmud-$MESHMUD_TAG-linux-amd64" "meshmud-$MESHMUD_TAG-linux-arm64" > "meshmud-$MESHMUD_TAG.sha256"
)
say "meshmud $MESHMUD_TAG built -> $OUT"
cat "$OUT/meshmud-$MESHMUD_TAG.sha256"
