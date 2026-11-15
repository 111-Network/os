#!/bin/sh
# Aggregate check for Layer 3: fmt + clippy + tests + musl target check.
set -eu
cd "$(dirname "$0")/.."
export PATH="$HOME/.cargo/bin:$PATH"
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo check --workspace --target x86_64-unknown-linux-musl
echo "check.sh: all green"
