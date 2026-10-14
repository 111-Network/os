#!/bin/sh
# Aggregate verification for os/bootstrap. Run from the layer root.
#
# Stages: fmt · clippy · musl check ×2 · test · shellcheck · notices (+ the Layer 5 build-script gates)
# · systemd-analyze · messaging (the mesh111 workspace: fmt/clippy/test/musl ×2, then the SQLCipher
# static link — os/messaging/README.md §19 L1, §20 G2) · presence sidecar · ui. Any failure stops the run;
# "ALL GREEN" is printed only when every stage passed (a stage whose tool is absent says SKIPPED).
#
# CARGO_TARGET_DIR is honoured (parallel builders keep private target dirs).
set -eu
cd "$(dirname "$0")/.."
export PATH="$HOME/.cargo/bin:$HOME/.local/bin:$PATH"
# musl cross toolchains (no-sudo installs live here; see scripts/cross-build.sh)
for tc in "$HOME"/.local/toolchains/*/bin; do
  [ -d "$tc" ] && PATH="$tc:$PATH"
done
export PATH

# SQLCipher + vendored OpenSSL (Layer 5, `mesh111`): libsqlite3-sys and openssl-src compile C through
# cc-rs, which finds the musl cross compilers above by their target prefix (x86_64-linux-musl-gcc,
# aarch64-linux-musl-gcc; AR likewise). The linkers are named here. This is the env
# ../messaging/scripts/check.sh uses, unchanged, so the node111 → mesh111 path dependency builds the
# same C objects in both workspaces. scripts/cross-build.sh pins CC/AR explicitly for release builds.
export CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_LINKER=x86_64-linux-musl-gcc
export CARGO_TARGET_AARCH64_UNKNOWN_LINUX_MUSL_LINKER=aarch64-linux-musl-gcc
MUSL=0
if command -v x86_64-linux-musl-gcc >/dev/null 2>&1 && command -v aarch64-linux-musl-gcc >/dev/null 2>&1; then
  MUSL=1
fi

echo "== fmt =="
cargo fmt --all -- --check
echo "== clippy (host) =="
cargo clippy --all-targets -- -D warnings
if [ "$MUSL" = 1 ]; then
  echo "== check (musl x86_64) =="
  cargo check --target x86_64-unknown-linux-musl
  echo "== check (musl aarch64) =="
  cargo check --target aarch64-unknown-linux-musl
else
  echo "!! musl cross toolchains absent (~/.local/toolchains: x86_64- and aarch64-linux-musl-gcc) — musl checks SKIPPED"
fi
echo "== test (host) =="
cargo test
echo "== shellcheck =="
if command -v shellcheck >/dev/null 2>&1; then
  find shim termux verify scripts -type f \( -name '*.sh' -o -name 'install.sh' \) -print0 2>/dev/null | xargs -0 -r shellcheck -s sh
  # the Layer 5 scripts this gate runs
  shellcheck -s sh ../messaging/scripts/check.sh ../messaging/sidecars/meshtastic-presence/run-tests.sh
else
  echo "shellcheck not found — SKIP"
fi
echo "== notices =="
# THIRD-PARTY-NOTICES.md records the lockfile hashes it was generated from; fail when one moved.
# Offline hash compare only. Fix: python3 -I scripts/gen-notices.py (needs cargo + pnpm, no network).
if command -v python3 >/dev/null 2>&1; then
  python3 -I scripts/gen-notices.py --check
  echo "== notices generator tests (Go / Python-venv / pin paths) =="
  python3 -I -B -m unittest discover -s scripts/notices/tests
else
  echo "python3 not found — notices check SKIPPED"
fi
echo "== build scripts (meshmud-build.sh / meshtastic-web-build.sh verify paths, hermetic) =="
# Pins, tarball/asset sha256, tag commit, go.mod/go.sum snapshots vs the notices data, the
# release-asset completeness rule, the "Go is CI-only" stop — against synthetic tarballs; no network.
if command -v git >/dev/null 2>&1; then
  sh scripts/notices/tests/build-scripts-gate.sh
else
  echo "git not found — build-scripts gate SKIPPED"
fi
echo "== systemd-analyze =="
if command -v systemd-analyze >/dev/null 2>&1 && ls systemd/*.service >/dev/null 2>&1; then
  # verify resolves units by name — point it at a temp unit dir (systemd 249
  # lacks `verify --root` and absolute-path FILE handling is version-dependent)
  SC_T=$(mktemp -d)
  cp systemd/*.service systemd/*.path "$SC_T/"
  SYSTEMD_UNIT_PATH="$SC_T" systemd-analyze verify 111node.service 111node-update.path 111node-update.service 2>&1 |
    grep -v "not executable\|not found\|netplan\|snapd\|Failed to create" || true
  rm -rf "$SC_T"
else
  echo "no units yet — SKIP"
fi
echo "== messaging (mesh111 workspace) =="
# fmt, clippy -D warnings, tests, musl check ×2 (SQLCipher + vendored OpenSSL compiled for both targets).
sh ../messaging/scripts/check.sh
if [ "$MUSL" = 1 ]; then
  # §20 G2: the static link, not only the compile. x86_64-musl test binaries are static and run here —
  # every store test opens, writes and prunes a real SQLCipher database inside a static musl binary.
  # aarch64 links only (no emulator on this box: running it is the recorded external gate).
  echo "== SQLCipher static link + run (mesh111 tests, musl x86_64) =="
  (cd ../messaging && cargo test --workspace --quiet --target x86_64-unknown-linux-musl)
  echo "== SQLCipher static link (mesh111 tests, musl aarch64; run = external gate G2) =="
  LINK_JSON=$(mktemp)
  (cd ../messaging && cargo test --workspace --no-run --target aarch64-unknown-linux-musl --message-format=json >"$LINK_JSON")
  AARCH_BINS=$(python3 -I -c 'import json, sys
for line in sys.stdin:
    try:
        m = json.loads(line)
    except ValueError:
        continue
    if m.get("reason") == "compiler-artifact" and m.get("executable"):
        print(m["executable"])' <"$LINK_JSON")
  rm -f "$LINK_JSON"
  [ -n "$AARCH_BINS" ] || { echo "FAIL: no aarch64 test binaries were linked" >&2; exit 1; }
  for b in $AARCH_BINS; do
    if readelf -lW "$b" | grep -q INTERP || readelf -dW "$b" 2>/dev/null | grep -q NEEDED; then
      echo "FAIL: $b is dynamically linked" >&2
      exit 1
    fi
    readelf -hW "$b" | grep -q 'AArch64' || { echo "FAIL: $b is not an AArch64 binary" >&2; exit 1; }
  done
  echo "ok: $(printf '%s\n' "$AARCH_BINS" | wc -l) aarch64 test binaries, static, SQLCipher + OpenSSL linked"
else
  echo "!! musl cross toolchains absent — SQLCipher static link SKIPPED"
fi
echo "== presence sidecar (meshtastic-presence: tests + venv lock structure) =="
if command -v python3 >/dev/null 2>&1; then
  sh ../messaging/sidecars/meshtastic-presence/run-tests.sh
  python3 -I ../messaging/sidecars/meshtastic-presence/gen_lock.py --verify
else
  echo "python3 not found — sidecar checks SKIPPED"
fi
echo "== ui =="
# Layer 4 dashboard (embedded into `111`): typecheck, vitest, vite build, size budget (D21 gate as is).
if command -v pnpm >/dev/null 2>&1; then
  if [ ! -f ../interface/ui/node_modules/.modules.yaml ]; then
    pnpm --dir ../interface/ui install --frozen-lockfile
  fi
  pnpm --dir ../interface/ui check
else
  echo "pnpm not found — UI checks SKIPPED (interface/ui: pnpm check = typecheck + test + build + budget)"
fi
echo "ALL GREEN"
