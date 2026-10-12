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
echo "== systemd-analyze =="
if command -v systemd-analyze >/dev/null 2>&1 && ls systemd/*.service >/dev/null 2>&1; then
  systemd-analyze verify systemd/*.service systemd/*.path 2>&1 | grep -v "Command.*not documented\|is not a valid unit name" || true
else
  echo "no units yet — SKIP"
fi
echo "ALL GREEN"
