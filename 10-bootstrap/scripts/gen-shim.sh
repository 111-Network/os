#!/bin/sh
# gen-shim — regenerate the pinned install shim for a release (spec §Artifacts:
# the shim is regenerated per release with fresh pins).
# Usage: scripts/gen-shim.sh CALVER ARTIFACTS_DIR [OUT_DIR]
set -eu

cd "$(dirname "$0")/.."
VER=${1:?usage: scripts/gen-shim.sh CALVER ARTIFACTS_DIR [OUT_DIR]}
ARTS=${2:?artifacts dir with 111-<triple> binaries}
OUT=${3:-dist}

SHA_X=$(sha256sum "$ARTS/111-x86_64-unknown-linux-musl" | cut -d' ' -f1)
SHA_A=$(sha256sum "$ARTS/111-aarch64-unknown-linux-musl" | cut -d' ' -f1)
MIN=$(grep -E '^pub const MIN_INSTALLER_VERSION' crates/node111/src/lib.rs | sed 's/.*"\(.*\)".*/\1/')
[ -n "$MIN" ] || { echo "could not read MIN_INSTALLER_VERSION from lib.rs" >&2; exit 1; }

mkdir -p "$OUT/v$VER"
sed -e "s/@@SHIM_VERSION@@/$VER/" \
	-e "s/@@SHA256_X86_64@@/$SHA_X/" \
	-e "s/@@SHA256_AARCH64@@/$SHA_A/" \
	-e "s/@@MIN_INSTALLER_VERSION@@/$MIN/" \
	shim/install.sh > "$OUT/v$VER/install.sh"

# Latest also at the top level (install.111.network/install.sh serves this).
cp "$OUT/v$VER/install.sh" "$OUT/install.sh"

# Shim's own pins for the independent channel (spec: sha256+minisig on the
# GitHub release page).
sha256sum "$OUT/v$VER/install.sh" > "$OUT/v$VER/install.sh.sha256"
echo "gen-shim OK → $OUT/v$VER/install.sh (min-version $MIN)"
