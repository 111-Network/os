#!/usr/bin/env bash
# 6-transport/verify/spine.sh — Layer 6 gate G4 (spec §16): the transport spine.
#
#   bash 6-transport/verify/spine.sh                 runs what it can, SKIPs the rest
#   REQUIRE_G4=1 bash 6-transport/verify/spine.sh    lock run: a missing prerequisite or a TODO step is a FAIL
#
# First cut (B4):
#   1. the spine's tests: golden renders per posture and stanza set (byte-exact, tests/fixtures/rns/*.golden),
#      the v1/v2 takeover seam, migration with the divergence line, rnstatus parser goldens, the posture apply
#      path, the join round trip, the PortTable claims (cargo test --test transport_spine; CARGO_TARGET_DIR and
#      CARGO_BUILD_JOBS are taken from the environment);
#   2. the modes, checked independently of the Rust asserts: the same test renders into a scratch root
#      (SPINE_KEEP_ROOT) and this script stats it: rns/ 0700, rns/config 0600, secrets/ 0700 with 0600 files,
#      bridge/lxmf/rns 0700 with a 0600 config, state/interfaces.toml 0600;
#   3. TODO V3 (netns, real rnsd from venvs/rns): rnstatus facts on the rendered dir, live attach/detach/reload,
#      the posture restart path with the helper, a double crash that must not partition, an RNode that cannot open
#      shows down while rnsd stays up, the second-AutoInterface case recorded, phone relay "Clients: 1" in two
#      netns, the QR-join flow between two 111d. Listed as SKIPPED here; under REQUIRE_G4=1 each is a FAIL.
#
# Nothing of ours stays running: cargo's test binary exits, the scratch root is removed on exit.
set -u
cd "$(dirname "$0")/../.." || exit 1
. 6-transport/verify/lib.sh
REPO=$(pwd)

command -v cargo >/dev/null 2>&1 || skip_or_fail G4 "cargo is not installed"

SCRATCH=$(mktemp -d "${TMPDIR:-/tmp}/111-spine.XXXXXX") || { fail "mktemp"; finish G4; }
cleanup() { rm -rf "$SCRATCH"; }
trap cleanup EXIT

# ---- 1. the spine's tests (golden renders, seams, apply path)
LOG="$SCRATCH/cargo.log"
if (cd "$REPO/1-bootstrap" && cargo test -q -p node111 --test transport_spine) >"$LOG" 2>&1; then
  ok "cargo test --test transport_spine: $(grep -E '^test result:' "$LOG" | tail -1)"
else
  tail -n 40 "$LOG" >&2
  fail "cargo test --test transport_spine"
fi

# ---- 2. modes on a rendered scratch root (independent stat check)
ROOT="$SCRATCH/root"
mkdir -p "$ROOT"
if (cd "$REPO/1-bootstrap" && SPINE_KEEP_ROOT="$ROOT" cargo test -q -p node111 --test transport_spine \
      modes_on_a_scratch_root) >"$LOG" 2>&1; then
  want_mode() { # PATH MODE
    if [ ! -e "$ROOT/$1" ]; then fail "$1 is missing"; return; fi
    if [ -L "$ROOT/$1" ]; then fail "$1 is a symlink"; return; fi
    got=$(stat -c '%a' "$ROOT/$1")
    if [ "$got" = "$2" ]; then ok "mode $2 $1"; else fail "$1 has mode $got, want $2"; fi
  }
  want_mode rns 700
  want_mode rns/config 600
  want_mode secrets 700
  want_mode secrets/rns-rpc-key 600
  want_mode bridge/lxmf 700
  want_mode bridge/lxmf/rns 700
  want_mode bridge/lxmf/rns/config 600
  want_mode state/interfaces.toml 600
  if head -n 20 "$ROOT/rns/config" | grep -q '^schema-version = 2$'; then
    ok "rns/config is schema-version 2"
  else
    fail "rns/config does not carry schema-version = 2"
  fi
  key=$(sed -n 's/^  rpc_key = \([0-9a-f]\{64\}\)$/\1/p' "$ROOT/rns/config")
  if [ -n "$key" ] && grep -q "^  rpc_key = $key\$" "$ROOT/bridge/lxmf/rns/config"; then
    ok "the helper's client config carries rnsd's rpc_key"
  else
    fail "rpc_key missing or different between rns/config and bridge/lxmf/rns/config"
  fi
  if grep -q '^\[\[' "$ROOT/bridge/lxmf/rns/config"; then
    fail "the helper's client config has an interface stanza"
  else
    ok "the helper's client config has no interfaces"
  fi
else
  tail -n 40 "$LOG" >&2
  fail "rendering the scratch root"
fi

# ---- 3. the real-rnsd steps (V3)
todo() { # STEP
  if [ "${REQUIRE_G4:-0}" = 1 ]; then
    fail "TODO V3: $* (not implemented)"
  else
    note "TODO V3: $* — SKIPPED"
  fi
}
if have_netns; then
  note "unprivileged netns available"
else
  note "unprivileged netns unavailable (unshare -Urnm): the V3 steps need it"
fi
todo "real rnsd on the rendered dir → rnstatus facts"
todo "live attach, detach, reload"
todo "posture restart: the helper stops, rnsd restarts, the helper returns; a double crash does not partition"
todo "an RNode stanza that cannot open shows down while rnsd stays up"
todo "the second-AutoInterface case, recorded"
todo "phone relay in two netns: Backbone + TCPClient, \"Clients: 1\""
todo "QR-join flow between two 111d"

finish G4
