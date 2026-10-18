#!/bin/sh
# Aggregate verification for os/10-bootstrap. Run from the layer root.
#
# Stages: fmt · clippy · musl check ×2 · test · shellcheck · notices (+ the Layer 5 build-script gates)
# · systemd-analyze · messaging (the mesh111 workspace: fmt/clippy/test/musl ×2) · G2, the SQLCipher
# static link (os/50-messaging/README.md §19 L1, §20 G2: mesh111's test binaries linked for both musl
# targets and asserted static, the mesh111 lib tests asserted to carry SQLCipher + the vendored
# OpenSSL, the x86_64 ones run) · presence sidecar · ui. Any failure stops the run.
#
# The last line says what ran: "ALL GREEN" only when every stage ran and passed. A stage whose tool
# is absent says SKIPPED and the run ends "GREEN (… SKIPPED — not a lock run)". G2 needs both musl
# cross toolchains; REQUIRE_G2=1 (lock runs, the release checklist) fails instead of skipping it.
#
# CARGO_TARGET_DIR is honoured (parallel builders keep private target dirs).
set -eu
cd "$(dirname "$0")/.."
export PATH="$HOME/.cargo/bin:$HOME/.local/bin:$PATH"
# musl cross toolchains, pinned CC/AR/linkers, the SQLite/OpenSSL escape hatches unset, and the
# static_elf / sqlcipher_linked / openssl_src_version assertions: scripts/musl-env.sh, the env
# scripts/cross-build.sh builds releases with (../50-messaging/scripts/check.sh exports the same CC/AR).
# shellcheck source=scripts/musl-env.sh
. scripts/musl-env.sh

SKIPPED=""
skipped() { # STAGE — record a stage that did not run (the last line names it)
  SKIPPED="$SKIPPED${SKIPPED:+, }$1"
}
MUSL=0
if musl_toolchains_present; then
  MUSL=1
elif [ "${REQUIRE_G2:-0}" = 1 ]; then
  echo "FAIL: REQUIRE_G2=1 but the musl cross toolchains are absent (~/.local/toolchains: x86_64- and aarch64-linux-musl-gcc/-ar), so G2 cannot run" >&2
  exit 1
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
  echo "!! musl cross toolchains absent (~/.local/toolchains: x86_64- and aarch64-linux-musl-gcc/-ar) — musl checks SKIPPED"
  skipped "musl checks"
fi
echo "== test (host) =="
cargo test
echo "== shellcheck =="
if command -v shellcheck >/dev/null 2>&1; then
  find shim termux verify scripts -type f \( -name '*.sh' -o -name 'install.sh' \) -print0 2>/dev/null | xargs -0 -r shellcheck -s sh
  # the Layer 5 scripts this gate runs
  shellcheck -s sh ../50-messaging/scripts/check.sh ../50-messaging/sidecars/meshtastic-presence/run-tests.sh
else
  echo "shellcheck not found — SKIPPED"
  skipped shellcheck
fi
echo "== gate assertions (scripts/musl-env.sh self-test: escape hatches, CC/AR pins, SQLCipher/OpenSSL/static asserts) =="
sh scripts/musl-env-test.sh
echo "== notices =="
# THIRD-PARTY-NOTICES.md records the lockfile hashes it was generated from; fail when one moved.
# Offline hash compare only. Fix: python3 -I scripts/gen-notices.py (needs cargo + pnpm, no network).
if command -v python3 >/dev/null 2>&1; then
  python3 -I scripts/gen-notices.py --check
  echo "== notices generator tests (Go / Python-venv / pin paths) =="
  python3 -I -B -m unittest discover -s scripts/notices/tests
else
  echo "python3 not found — notices check SKIPPED"
  skipped notices
fi
echo "== build scripts (meshmud-build.sh / meshtastic-web-build.sh verify paths, hermetic) =="
# Pins, tarball/asset sha256, tag commit, go.mod/go.sum snapshots vs the notices data, the
# release-asset completeness rule, the "Go is CI-only" stop — against synthetic tarballs; no network.
if command -v git >/dev/null 2>&1; then
  sh scripts/notices/tests/build-scripts-gate.sh
else
  echo "git not found — build-scripts gate SKIPPED"
  skipped "build-scripts gate"
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
  echo "systemd-analyze or the units absent — SKIPPED"
  skipped systemd-analyze
fi
echo "== messaging (mesh111 workspace) =="
# fmt, clippy -D warnings, tests, musl check ×2 (SQLCipher + vendored OpenSSL compiled for both targets).
sh ../50-messaging/scripts/check.sh

# g2_link TRIPLE MACHINE — link (not run) mesh111's test binaries for TRIPLE; every one must be a
# static MACHINE ELF, and the mesh111 lib-test binary (the store tests) must carry SQLCipher's codec
# and the vendored OpenSSL. Sets G2_BINS (binaries) and G2_CIPHER (binaries carrying both).
g2_link() {
  g2_json=$(mktemp)
  (cd ../50-messaging && cargo test --workspace --no-run --target "$1" --message-format=json >"$g2_json")
  g2_list=$(mktemp)
  python3 -I -c 'import json, sys
for line in sys.stdin:
    try:
        m = json.loads(line)
    except ValueError:
        continue
    if m.get("reason") == "compiler-artifact" and m.get("executable"):
        kinds = m["target"]["kind"]
        kind = "lib" if "lib" in kinds else kinds[0]
        print(kind, m["target"]["name"], m["executable"])' <"$g2_json" >"$g2_list"
  rm -f "$g2_json"
  G2_BINS=0
  G2_CIPHER=0
  g2_lib=0
  while read -r kind name bin; do
    G2_BINS=$((G2_BINS + 1))
    static_elf "$bin" "$2"
    if [ "$kind" = lib ] && [ "$name" = mesh111 ]; then
      sqlcipher_linked "$bin" "$OSSL_VER"
      g2_lib=1
    fi
    if sqlcipher_linked "$bin" "$OSSL_VER" 2>/dev/null; then
      G2_CIPHER=$((G2_CIPHER + 1))
    fi
  done <"$g2_list"
  rm -f "$g2_list"
  [ "$G2_BINS" -gt 0 ] || { echo "FAIL: no $1 test binaries were linked" >&2; exit 1; }
  [ "$g2_lib" = 1 ] || { echo "FAIL: the mesh111 lib-test binary is not among the $1 test binaries" >&2; exit 1; }
}

if [ "$MUSL" = 1 ]; then
  # The OpenSSL release openssl-src vendors (its `+<ver>` crate-version suffix), asserted in the binaries.
  OSSL_VER=$(openssl_src_version ../50-messaging/Cargo.lock)
  [ -n "$OSSL_VER" ] || { echo "FAIL: ../50-messaging/Cargo.lock lists no openssl-src (+<OpenSSL version>)" >&2; exit 1; }
  # §20 G2: the static link, not only the compile. x86_64-musl test binaries are static and run here —
  # every store test opens, writes and prunes a real SQLCipher database inside a static musl binary.
  # aarch64 links only (no emulator on this box: running it is the recorded external gate).
  echo "== G2: SQLCipher static link + run (mesh111 tests, musl x86_64) =="
  g2_link x86_64-unknown-linux-musl X86-64
  echo "ok: $G2_BINS x86_64 test binaries static X86-64 (no INTERP/NEEDED); $G2_CIPHER carry SQLCipher (cipher_plaintext_header_size) + OpenSSL $OSSL_VER, the mesh111 lib tests among them"
  (cd ../50-messaging && cargo test --workspace --quiet --target x86_64-unknown-linux-musl)
  echo "== G2: SQLCipher static link (mesh111 tests, musl aarch64; run = external gate G2) =="
  g2_link aarch64-unknown-linux-musl AArch64
  echo "ok: $G2_BINS aarch64 test binaries static AArch64 (no INTERP/NEEDED); $G2_CIPHER carry SQLCipher (cipher_plaintext_header_size) + OpenSSL $OSSL_VER, the mesh111 lib tests among them"
else
  echo "!! musl cross toolchains absent — G2 (SQLCipher static link) SKIPPED; REQUIRE_G2=1 makes this a failure"
fi
echo "== presence sidecar (meshtastic-presence: tests + venv lock structure) =="
if command -v python3 >/dev/null 2>&1; then
  sh ../50-messaging/sidecars/meshtastic-presence/run-tests.sh
  python3 -I ../50-messaging/sidecars/meshtastic-presence/gen_lock.py --verify
else
  echo "python3 not found — sidecar checks SKIPPED"
  skipped "presence sidecar"
fi
echo "== ui =="
# Layer 4 dashboard (embedded into `111`): typecheck, vitest, vite build, size budget (D21 gate as is).
if command -v pnpm >/dev/null 2>&1; then
  if [ ! -f ../40-interface/ui/node_modules/.modules.yaml ]; then
    pnpm --dir ../40-interface/ui install --frozen-lockfile
  fi
  pnpm --dir ../40-interface/ui check
else
  echo "pnpm not found — UI checks SKIPPED (40-interface/ui: pnpm check = typecheck + test + build + budget)"
  skipped ui
fi
if [ "$MUSL" != 1 ]; then
  echo "GREEN (G2 SKIPPED — not a lock run)${SKIPPED:+; also skipped: $SKIPPED}"
elif [ -n "$SKIPPED" ]; then
  echo "GREEN ($SKIPPED SKIPPED — not a lock run)"
else
  echo "ALL GREEN"
fi
