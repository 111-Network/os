#!/usr/bin/env bash
# run.sh -- build and run the Monocypher C cross-vector harness (T9).
#
#   bash os/20-identity/monocypher-harness/run.sh [--sanitize] [--log FILE] [--no-supplement]
#
# Steps: (1) verify the vendored Monocypher files against SHA256SUMS,
# (2) build harness.c + vendored monocypher with
#     cc -std=c99 -O2 -Wall -Wextra -Werror   (CC overridable),
# (3) assert every test-vectors/*.json is accounted for by the harness,
# (4) run it over the frozen vectors; exit non-zero on any mismatch.
# Stdlib/libc only. No network. Build products go to a throw-away temp dir.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
vectors="$(cd "$here/../test-vectors" && pwd)"
cc="${CC:-cc}"
sanitize=0
log=""
supp=1
while [ $# -gt 0 ]; do
  case "$1" in
    --sanitize) sanitize=1 ;;
    --log) log="${2:?--log needs a file}"; shift ;;
    --no-supplement) supp=0 ;;
    *) echo "usage: run.sh [--sanitize] [--log FILE] [--no-supplement]" >&2; exit 2 ;;
  esac
  shift
done

if [ -n "$log" ]; then : > "$log"; fi
say() { printf '%s\n' "$*"; if [ -n "$log" ]; then printf '%s\n' "$*" >> "$log"; fi; }

# (1) hash-pin check of the vendored sources
( cd "$here" && sha256sum --check --strict --quiet SHA256SUMS ) \
  || { echo "run.sh: vendored Monocypher files do not match SHA256SUMS" >&2; exit 1; }
say "vendor: SHA256SUMS verified (Monocypher 4.0.3, see VENDOR.md)"

# (2) build
build="$(mktemp -d "${TMPDIR:-/tmp}/monocypher-harness.XXXXXX")"
trap 'rm -rf "$build"' EXIT
cflags=(-std=c99 -O2 -Wall -Wextra -Werror)
if [ "$sanitize" = 1 ]; then cflags+=(-g -fsanitize=address,undefined -fno-sanitize-recover=all -fno-omit-frame-pointer); fi
say "cc: $("$cc" --version | head -n1)  flags: ${cflags[*]}"
"$cc" "${cflags[@]}" -I"$here/vendor" -I"$here/vendor/optional" \
  -o "$build/harness" \
  "$here/harness.c" "$here/vendor/monocypher.c" "$here/vendor/optional/monocypher-ed25519.c"

# (3) every vector file must be accounted for (consumed or n/a with a reason)
found="$(cd "$vectors" && ls -1 ./*.json | sed 's|^\./||' | LC_ALL=C sort)"
known="$("$build/harness" --list | awk '{print $2}' | LC_ALL=C sort)"
if [ "$found" != "$known" ]; then
  echo "run.sh: test-vectors/*.json and the harness inventory disagree:" >&2
  diff <(printf '%s\n' "$found") <(printf '%s\n' "$known") >&2 || true
  exit 1
fi
say "inventory: all $(printf '%s\n' "$found" | wc -l | tr -d ' ') vector files accounted for"

# (4) run
args=("$vectors")
if [ "$supp" = 1 ] && [ -f "$here/supplement/cofactor-edges.json" ]; then args=(--supplement "$here/supplement/cofactor-edges.json" "$vectors"); fi
set +e
if [ -n "$log" ]; then "$build/harness" "${args[@]}" 2>&1 | tee -a "$log"; rc=${PIPESTATUS[0]}
else "$build/harness" "${args[@]}"; rc=$?; fi
set -e
if [ "$rc" -ne 0 ]; then echo "run.sh: HARNESS FAILED (exit $rc)" >&2; exit "$rc"; fi
say "run.sh: OK -- every file PASS"
