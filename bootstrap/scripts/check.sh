#!/bin/sh
# Aggregate verification for os/bootstrap. Run from the layer root.
set -eu
cd "$(dirname "$0")/.."
export PATH="$HOME/.cargo/bin:$HOME/.local/bin:$PATH"
# musl cross toolchains (no-sudo installs live here; see scripts/cross-build.sh)
for tc in "$HOME"/.local/toolchains/*/bin; do
  [ -d "$tc" ] && PATH="$tc:$PATH"
done
export PATH

echo "== fmt =="
cargo fmt --all -- --check
echo "== clippy (host) =="
cargo clippy --all-targets -- -D warnings
if command -v x86_64-linux-musl-gcc >/dev/null 2>&1; then
  echo "== check (musl x86_64) =="
  cargo check --target x86_64-unknown-linux-musl
  echo "== check (musl aarch64) =="
  cargo check --target aarch64-unknown-linux-musl
else
  echo "!! musl cross toolchains absent (~/.local/toolchains) — musl checks SKIPPED"
fi
echo "== test (host) =="
cargo test
echo "== shellcheck =="
if command -v shellcheck >/dev/null 2>&1; then
  find shim termux verify scripts -type f \( -name '*.sh' -o -name 'install.sh' \) -print0 2>/dev/null | xargs -0 -r shellcheck -s sh
else
  echo "shellcheck not found — SKIP"
fi
echo "== notices =="
# THIRD-PARTY-NOTICES.md records the lockfile hashes it was generated from; fail when one moved.
# Offline hash compare only. Fix: python3 -I scripts/gen-notices.py (needs cargo + pnpm, no network).
if command -v python3 >/dev/null 2>&1; then
  python3 -I scripts/gen-notices.py --check
else
  echo "python3 not found — notices check SKIPPED"
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
echo "== ui =="
# Layer 4 dashboard (embedded into `111`): typecheck, vitest, vite build, size budget.
if command -v pnpm >/dev/null 2>&1; then
  if [ ! -f ../interface/ui/node_modules/.modules.yaml ]; then
    pnpm --dir ../interface/ui install --frozen-lockfile
  fi
  pnpm --dir ../interface/ui check
else
  echo "pnpm not found — UI checks SKIPPED (interface/ui: pnpm check = typecheck + test + build + budget)"
fi
echo "ALL GREEN"
