#!/bin/sh
# Aggregate check for Layer 5 (mesh111): fmt + clippy + tests + musl checks.
# SQLCipher + vendored OpenSSL are C code: the musl checks need the cross
# toolchains at ~/.local/toolchains (see bootstrap/scripts/cross-build.sh).
set -eu
cd "$(dirname "$0")/.."
export PATH="$HOME/.cargo/bin:$PATH"
for tc in "$HOME"/.local/toolchains/*/bin; do
  [ -d "$tc" ] && PATH="$tc:$PATH"
done
export PATH
export CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_LINKER=x86_64-linux-musl-gcc
export CARGO_TARGET_AARCH64_UNKNOWN_LINUX_MUSL_LINKER=aarch64-linux-musl-gcc

echo "== fmt =="
cargo fmt --all -- --check
echo "== clippy =="
cargo clippy --workspace --all-targets -- -D warnings
echo "== test =="
cargo test --workspace
if command -v x86_64-linux-musl-gcc >/dev/null 2>&1; then
  echo "== check (musl x86_64) =="
  cargo check --workspace --target x86_64-unknown-linux-musl
  echo "== check (musl aarch64) =="
  cargo check --workspace --target aarch64-unknown-linux-musl
else
  echo "!! musl cross toolchains absent (~/.local/toolchains) — musl checks SKIPPED"
fi
echo "check.sh: all green"
