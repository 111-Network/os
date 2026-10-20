#!/usr/bin/env bash
# 60-transport/verify/lib.sh — shared helpers for the Layer 6 gate scripts (spec §16).
#
# Convention (from 10-bootstrap/scripts/check.sh REQUIRE_G2 and 40-interface/verify/smoke.sh):
# every check prints `ok:` or `FAIL:` (`note:` is information only); a script exits 1 on any
# FAIL. A gate whose prerequisites are absent (unprivileged netns, venvs/rns, a built binary)
# prints `SKIPPED` and exits 0 — unless its REQUIRE_<GATE> variable is set (lock runs), in which
# case the missing prerequisite is a FAIL. Nothing of ours stays running after a script ends.
set -u
FAILS=0
ok()   { printf 'ok: %s\n' "$*"; }
note() { printf 'note: %s\n' "$*"; }
fail() { printf 'FAIL: %s\n' "$*" >&2; FAILS=$((FAILS + 1)); }
# skip_or_fail GATE REASON — SKIPPED (exit 0) unless REQUIRE_<GATE>=1, then FAIL (exit 1).
skip_or_fail() {
  gate=$1; shift
  var="REQUIRE_$gate"
  if [ "${!var:-0}" = 1 ]; then
    printf 'FAIL: %s: %s (REQUIRE_%s=1)\n' "$gate" "$*" "$gate" >&2
    exit 1
  fi
  printf '%s SKIPPED: %s\n' "$gate" "$*"
  exit 0
}
have_netns() { unshare -Urnm true >/dev/null 2>&1; }
finish() { # GATE — the last line names the result
  if [ "$FAILS" -gt 0 ]; then printf '%s: %d FAIL\n' "$1" "$FAILS" >&2; exit 1; fi
  printf '%s GREEN\n' "$1"
}
