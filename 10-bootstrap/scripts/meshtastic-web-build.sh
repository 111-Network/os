#!/bin/sh
# meshtastic-web-build: produce the Meshtastic web client bundle that the dashboard hosts as a panel
# (spec 50-messaging/README.md section 13, section 14; assets/panels/meshtastic.toml `bundle`).
#
# The bundle is GPL-3.0-only and is served UNMODIFIED from its own directory, never embedded in the
# `111` binary (GPL boundary). This script only ever fetches the PINNED upstream tag; nothing here
# runs on a node and nothing downloads at runtime. Run it in the release workflow (CI), or on a dev
# box with git + pnpm + node when you move the pin.
#
# Modes (--mode):
#   auto    (default) use the upstream release asset (build.tar) when the pin names one, its sha256
#           matches and it is COMPLETE; otherwise build from the tag with pnpm.
#   asset   the release asset only; fail if it is missing, altered or incomplete.
#   source  always build from the tag: git clone of the pinned tag (HEAD must equal the pinned commit),
#           `pnpm install --frozen-lockfile --ignore-scripts`, `pnpm run build` in apps/web.
#
# Why "complete" matters (checked for v2.7.2): upstream's build.tar is made by `gzipper` with a fixed
# extension list, so it silently leaves out the sqlite `.wasm` the client loads for its message store
# and the `.woff2` fonts. A bundle missing them loads but breaks. The completeness gate rejects any
# asset whose JS/CSS/HTML point at a same-origin .wasm/.woff/.woff2/.ttf/.otf/.eot file that is not in
# it; `auto` then builds from the tag. A source build from a git checkout of the tag is byte-identical
# to upstream's own files (same hashed chunk names) and adds the missing ones; --cross-check proves it.
#
# usage: scripts/meshtastic-web-build.sh [--mode auto|asset|source] [--asset FILE] [--cross-check]
#                                        [--verify-only] [--licenses FILE] [--out DIR] [--work DIR]
#   --asset FILE     use a local copy of the release asset instead of downloading it
#   --cross-check    build from source AND unpack the asset; every asset file must equal the built file
#   --verify-only    download (or take --asset) and verify the asset only: sha256, safe paths,
#                    completeness. Needs no git/pnpm/node. Prints what `auto` would do.
#   --licenses FILE  (source builds) also write `pnpm licenses list --prod --json` for apps/web to FILE
#   --out DIR        output directory (default: dist/meshtastic-web/<tag>)
#   --work DIR       scratch directory (default: mktemp -d, removed on exit)
#
# Output (under --out):
#   meshtastic-web/                    the served directory: plain files, no .gz
#   meshtastic-web-<tag>.tree          `sha256  path` for every file, sorted (C locale)
#   meshtastic-web-<tag>.sha256        sha256 of the .tree file: THE bundle hash for the manifest. The build
#                                      must reproduce WEB_TREE_SHA256 below (two source builds and the
#                                      upstream asset agree byte for byte); a different tree fails the run
#                                      until a reviewer re-pins it.
#   meshtastic-web-<tag>.tar           deterministic tar of the directory (+ .tar.sha256)
#   meshtastic-web-<tag>.SOURCE        mode, tag, commit, asset sha256 (the GPL source offer names these)
#   LICENSE.meshtastic-web             upstream GPL-3.0 text, ships beside the bundle (not inside it)
#
# Pins: ONE assignment per line, double-quoted. gen-notices.py reads these exact lines.
WEB_REPO="https://github.com/meshtastic/web"
WEB_TAG="v2.7.2"
WEB_COMMIT="ee5243a2059126ecd799927d21e052b5f5745974"
WEB_ASSET_NAME="build.tar"
WEB_ASSET_SHA256="62657b85b4c24af4d44da2932b64143abdbf6e65a79fcb51fd0801d9540616e2"
WEB_TREE_SHA256="39f54e5eb79ecd398c17ef66a48b58f38cbe72102ab8980641f660d01f46e28b"
WEB_NODE_MIN="22"

set -eu

MODE=auto
ASSET=""
CROSS=0
VERIFY_ONLY=0
LICENSES=""
OUT=""
WORK=""
MAX_FILE_BYTES=67108864

die() { printf '%s\n' "meshtastic-web-build: FAIL: $*" >&2; exit 1; }
say() { printf '%s\n' "== $*"; }

while [ $# -gt 0 ]; do
  case "$1" in
    --mode) shift; [ $# -gt 0 ] || die "--mode needs a value"; MODE=$1 ;;
    --asset) shift; [ $# -gt 0 ] || die "--asset needs a file"; ASSET=$1 ;;
    --cross-check) CROSS=1 ;;
    --verify-only) VERIFY_ONLY=1 ;;
    --licenses) shift; [ $# -gt 0 ] || die "--licenses needs a file"; LICENSES=$1 ;;
    --out) shift; [ $# -gt 0 ] || die "--out needs a directory"; OUT=$1 ;;
    --work) shift; [ $# -gt 0 ] || die "--work needs a directory"; WORK=$1 ;;
    -h|--help) sed -n '2,45p' "$0"; exit 0 ;;
    *) die "unknown argument: $1" ;;
  esac
  shift
done
case "$MODE" in auto|asset|source) ;; *) die "--mode must be auto, asset or source" ;; esac

need_pin() { # NAME VALUE
  case "$2" in
    ""|*TODO*) die "pin $1 is not set" ;;
  esac
}
need_pin WEB_TAG "$WEB_TAG"
need_pin WEB_COMMIT "$WEB_COMMIT"
# WEB_ASSET_SHA256 may be empty: "upstream ships no asset for this tag", then only a source build is possible.

for tool in sha256sum tar gzip sed awk find sort mktemp head cmp grep; do
  command -v "$tool" >/dev/null 2>&1 || die "missing tool: $tool"
done

[ -n "$OUT" ] || OUT="dist/meshtastic-web/$WEB_TAG"
if [ -z "$WORK" ]; then
  WORK=$(mktemp -d "${TMPDIR:-/tmp}/meshtastic-web-build.XXXXXX")
  trap 'rm -rf "$WORK"' EXIT
else
  mkdir -p "$WORK"
fi

# --- helpers ---------------------------------------------------------------------------------
fetch() { # URL FILE
  command -v curl >/dev/null 2>&1 || die "missing tool: curl"
  curl --fail --silent --show-error --location --proto '=https' --tlsv1.2 --max-time 300 \
    --output "$2" "$1"
}

# Refuse links, devices, absolute paths and ".." before anything is extracted.
safe_tar() { # TARFILE
  if tar -tvf "$1" | awk '$1 !~ /^[-d]/ {bad=1} END {exit !bad}'; then
    die "$1 contains links or special files"
  fi
  if tar -tf "$1" | awk '/^\// || /(^|\/)\.\.(\/|$)/ {bad=1} END {exit !bad}'; then
    die "$1 contains an absolute or parent path"
  fi
}

# Upstream's asset is entirely pre-gzipped (index.html.gz ...). Unpack to plain files, bounded.
gunzip_tree() { # DIR
  find "$1" -type f -name '*.gz' | while read -r f; do
    plain=${f%.gz}
    gzip -dc "$f" | head -c "$((MAX_FILE_BYTES + 1))" > "$plain"
    size=$(wc -c < "$plain")
    [ "$size" -le "$MAX_FILE_BYTES" ] || die "$f expands beyond $MAX_FILE_BYTES bytes"
    rm -f "$f"
  done
}

# Same-origin references in the bundle's JS/CSS/HTML to binary types that must exist as files.
missing_refs() { # DIR -> prints one missing reference per line
  (
    cd "$1"
    grep -rhoE "[\"'\`(=]/[A-Za-z0-9._/-]+\.(wasm|woff2?|ttf|otf|eot)" \
      --include='*.js' --include='*.css' --include='*.html' --include='*.webmanifest' . 2>/dev/null |
      sed 's/^.//' | sort -u | while read -r ref; do
        [ -f ".$ref" ] || printf '%s\n' "$ref"
      done
  )
}

tree_manifest() { # DIR OUTFILE
  (
    cd "$1"
    find . -type f -print0 | LC_ALL=C sort -z | xargs -0 sha256sum | sed 's#  \./#  #'
  ) > "$2"
}

# --- 1. the release asset (if pinned) ---------------------------------------------------------
ASSET_DIR="$WORK/asset-tree"
ASSET_OK=0
ASSET_WHY=""
if [ "$MODE" != source ]; then
  if [ -z "$WEB_ASSET_SHA256" ]; then
    ASSET_WHY="the pin names no release asset"
  else
    if [ -z "$ASSET" ]; then
      ASSET="$WORK/$WEB_ASSET_NAME"
      say "fetch $WEB_REPO/releases/download/$WEB_TAG/$WEB_ASSET_NAME"
      if ! fetch "$WEB_REPO/releases/download/$WEB_TAG/$WEB_ASSET_NAME" "$ASSET"; then
        ASSET_WHY="the asset could not be downloaded"
        ASSET=""
      fi
    fi
    if [ -n "$ASSET" ]; then
      [ -f "$ASSET" ] || die "asset not found: $ASSET"
      say "verify asset sha256"
      GOT=$(sha256sum "$ASSET" | awk '{print $1}')
      [ "$GOT" = "$WEB_ASSET_SHA256" ] || die "asset sha256 mismatch: pinned $WEB_ASSET_SHA256, got $GOT"
      safe_tar "$ASSET"
      rm -rf "$ASSET_DIR"
      mkdir -p "$ASSET_DIR"
      tar -xf "$ASSET" -C "$ASSET_DIR" --no-same-owner --no-same-permissions
      gunzip_tree "$ASSET_DIR"
      if [ ! -f "$ASSET_DIR/index.html" ]; then
        ASSET_WHY="the asset has no index.html"
      else
        MISSING=$(missing_refs "$ASSET_DIR")
        if [ -n "$MISSING" ]; then
          ASSET_WHY="the asset is incomplete; the bundle points at files that are not in it: $(printf '%s' "$MISSING" | tr '\n' ' ')"
        else
          ASSET_OK=1
        fi
      fi
    fi
  fi
  if [ "$ASSET_OK" = 1 ]; then
    say "release asset verified and complete"
  else
    say "release asset not usable: $ASSET_WHY"
  fi
fi

if [ "$MODE" = asset ] && [ "$ASSET_OK" != 1 ]; then
  die "--mode asset: $ASSET_WHY"
fi

if [ "$VERIFY_ONLY" = 1 ]; then
  if [ "$MODE" = source ]; then die "--verify-only has nothing to verify with --mode source"; fi
  if [ "$ASSET_OK" = 1 ]; then say "verify-only: auto would use the release asset"; else say "verify-only: auto would build from the tag ($WEB_TAG, $WEB_COMMIT)"; fi
  exit 0
fi

# --- 2. a build from the pinned tag ------------------------------------------------------------
SRC_DIR="$WORK/source-tree"
build_from_source() {
  for tool in git pnpm node; do
    command -v "$tool" >/dev/null 2>&1 || die "missing tool for a source build: $tool"
  done
  NODE_MAJOR=$(node --version | sed 's/^v\([0-9]*\).*/\1/')
  [ "$NODE_MAJOR" -ge "$WEB_NODE_MIN" ] || die "node $NODE_MAJOR is older than the required $WEB_NODE_MIN"
  say "clone $WEB_REPO at $WEB_TAG"
  rm -rf "$WORK/web"
  git clone --quiet --depth 1 --branch "$WEB_TAG" "$WEB_REPO.git" "$WORK/web" || die "git clone failed"
  HEAD_ID=$(git -C "$WORK/web" rev-parse HEAD)
  [ "$HEAD_ID" = "$WEB_COMMIT" ] || die "tag $WEB_TAG points at $HEAD_ID, pinned $WEB_COMMIT"
  # the upstream vite config stamps `git rev-parse --short HEAD` and `git describe --tags` into the
  # bundle, which is why this is a git checkout and not a tarball
  say "pnpm install --frozen-lockfile --ignore-scripts"
  ( cd "$WORK/web" && CI=true pnpm install --frozen-lockfile --ignore-scripts ) || die "pnpm install failed"
  say "pnpm run build (apps/web)"
  ( cd "$WORK/web/apps/web" && CI=true pnpm run build ) || die "pnpm build failed"
  [ -f "$WORK/web/apps/web/dist/index.html" ] || die "the build produced no dist/index.html"
  rm -rf "$SRC_DIR"
  cp -R "$WORK/web/apps/web/dist" "$SRC_DIR"
  if [ -n "$LICENSES" ]; then
    say "pnpm licenses (production dependencies of apps/web, workspace-wide; review before use)"
    ( cd "$WORK/web/apps/web" && pnpm licenses list --prod --json ) > "$LICENSES" || die "pnpm licenses failed"
  fi
  cp "$WORK/web/LICENSE" "$WORK/LICENSE.meshtastic-web"
}

USED=""
if [ "$CROSS" = 1 ]; then
  [ "$ASSET_OK" = 1 ] || [ -f "$ASSET_DIR/index.html" ] || die "--cross-check needs an unpacked release asset ($ASSET_WHY)"
  build_from_source
  say "cross-check: every file of the release asset must equal the file built from the tag"
  ( cd "$ASSET_DIR" && find . -type f -print ) | while read -r f; do
    if ! cmp -s "$ASSET_DIR/$f" "$SRC_DIR/$f"; then
      printf '%s\n' "differs or missing in the build: $f" >&2
      echo x > "$WORK/cross-diff"
    fi
  done
  [ ! -f "$WORK/cross-diff" ] || die "cross-check failed"
  say "cross-check ok (the build also holds $(find "$SRC_DIR" -type f | wc -l) files, the asset $(find "$ASSET_DIR" -type f | wc -l))"
  FINAL="$SRC_DIR"
  USED=source
elif [ "$ASSET_OK" = 1 ]; then
  FINAL="$ASSET_DIR"
  USED=asset
  # the asset carries no license file; take the commit-pinned one
  fetch "https://raw.githubusercontent.com/meshtastic/web/$WEB_COMMIT/LICENSE" "$WORK/LICENSE.meshtastic-web" ||
    die "cannot fetch the pinned LICENSE"
  grep -q 'GNU GENERAL PUBLIC LICENSE' "$WORK/LICENSE.meshtastic-web" || die "the fetched LICENSE is not the GPL"
else
  build_from_source
  FINAL="$SRC_DIR"
  USED=source
fi

# --- 3. finish: no links, a missing-ref report, tree hash, deterministic tar -------------------
if find "$FINAL" ! -type f ! -type d | grep -q .; then die "the bundle contains links or special files"; fi
if [ ! -f "$FINAL/index.html" ]; then die "the bundle has no index.html"; fi
MISSING=$(missing_refs "$FINAL")
[ -z "$MISSING" ] || die "the final bundle still points at missing files: $(printf '%s' "$MISSING" | tr '\n' ' ')"

tree_manifest "$FINAL" "$WORK/bundle.tree"
GOT_TREE=$(sha256sum "$WORK/bundle.tree" | awk '{print $1}')
if [ -n "$WEB_TREE_SHA256" ] && [ "$GOT_TREE" != "$WEB_TREE_SHA256" ]; then
  die "bundle tree hash $GOT_TREE differs from the pinned WEB_TREE_SHA256 $WEB_TREE_SHA256 (toolchain drift or a moved tag): review the diff against the previous bundle, then re-pin"
fi

mkdir -p "$OUT"
OUT=$(cd "$OUT" && pwd)
rm -rf "$OUT/meshtastic-web"
cp -R "$FINAL" "$OUT/meshtastic-web"
find "$OUT/meshtastic-web" -type d -exec chmod 755 {} +
find "$OUT/meshtastic-web" -type f -exec chmod 644 {} +

cp "$WORK/bundle.tree" "$OUT/meshtastic-web-$WEB_TAG.tree"
( cd "$OUT" && sha256sum "meshtastic-web-$WEB_TAG.tree" > "meshtastic-web-$WEB_TAG.sha256" )
tar --sort=name --owner=0 --group=0 --numeric-owner --mtime=@0 --format=gnu \
  -cf "$OUT/meshtastic-web-$WEB_TAG.tar" -C "$OUT/meshtastic-web" .
( cd "$OUT" && sha256sum "meshtastic-web-$WEB_TAG.tar" > "meshtastic-web-$WEB_TAG.tar.sha256" )

{
  echo "project=$WEB_REPO"
  echo "tag=$WEB_TAG"
  echo "commit=$WEB_COMMIT"
  echo "mode=$USED"
  echo "release_asset=$WEB_REPO/releases/download/$WEB_TAG/$WEB_ASSET_NAME"
  echo "release_asset_sha256=$WEB_ASSET_SHA256"
  echo "release_asset_note=${ASSET_WHY:-complete}"
} > "$OUT/meshtastic-web-$WEB_TAG.SOURCE"
cp "$WORK/LICENSE.meshtastic-web" "$OUT/LICENSE.meshtastic-web"

say "meshtastic web $WEB_TAG ($USED) -> $OUT/meshtastic-web"
cat "$OUT/meshtastic-web-$WEB_TAG.sha256"
