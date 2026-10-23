#!/usr/bin/env bash
# 6-transport/verify/rns-venv.sh — the gate's Reticulum venv (spec §16 G3/G4/G5), shared by the gate scripts.
#
# Builds one Python venv from the LXMF helper's hash-locked closure
# (6-transport/sidecars/lxmf-sidecar/lxmf-sidecar-lock.txt: rns 1.5.7, lxmf 1.2.1, pyserial, cryptography; CPython
# >= 3.11) with `pip download --require-hashes --only-binary=:all:` then `pip install --require-hashes --no-index`,
# so the gate's rnsd, rnstatus and rnodeconf are the pinned ones. The production split (venvs/rns from the
# release's rns-lock.txt) is not reproduced: the lxmf lock carries rns, so this one venv stands in for both and a
# gate root links <root>/venvs/rns/bin/{rnsd,rnstatus} to it (rnstatus::program(root) reads
# <root>/venvs/rns/bin/rnstatus). Nothing here runs at a daemon's runtime: the gate builds it before any daemon
# starts (PyPI through the box's proxy, once per lock).
#
# Sourced:
#   . 6-transport/verify/rns-venv.sh
#   rns_venv_ensure DIR     → 0 and RNS_VENV=DIR/venv, RNS_WHEELS=DIR/wheels, RNS_LOCK, RNS_LOCKSHA;
#                             1 with RNS_VENV_ERR set (PyPI unreachable, python3 < 3.11, a hash mismatch).
#                             Reused while DIR/lock.sha256 names this lock and the venv imports RNS + LXMF.
#   rns_venv_remove DIR     → deletes what rns_venv_ensure made under DIR (and DIR when empty).
# Run:
#   bash 6-transport/verify/rns-venv.sh [DIR]   builds (or reuses) and prints the venv path; exit 1 on failure.
#
# DIR defaults to ${RNS_VENV_DIR:-${TMPDIR:-/tmp}/111-rns-venv}. V2's lxmf.sh keeps its own cache (G3_CACHE); this
# helper never touches it.

RNS_VENV_HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
RNS_LOCK=${RNS_LOCK:-$RNS_VENV_HERE/../sidecars/lxmf-sidecar/lxmf-sidecar-lock.txt}

rns_venv_ensure() {
  local dir=${1:-${RNS_VENV_DIR:-${TMPDIR:-/tmp}/111-rns-venv}}
  RNS_VENV=$dir/venv
  RNS_WHEELS=$dir/wheels
  RNS_VENV_ERR=""
  if [ ! -r "$RNS_LOCK" ]; then
    RNS_VENV_ERR="the lock $RNS_LOCK is missing"
    return 1
  fi
  if ! command -v python3 >/dev/null 2>&1; then
    RNS_VENV_ERR="python3 is not installed"
    return 1
  fi
  if ! python3 -c 'import sys; sys.exit(sys.version_info < (3, 11))' 2>/dev/null; then
    RNS_VENV_ERR="python3 is older than 3.11 (the lock's floor)"
    return 1
  fi
  RNS_LOCKSHA=$(sha256sum "$RNS_LOCK" | cut -c1-64)
  mkdir -p "$dir" || {
    RNS_VENV_ERR="cannot create $dir"
    return 1
  }
  if [ "$(cat "$dir/lock.sha256" 2>/dev/null)" = "$RNS_LOCKSHA" ] && [ -x "$RNS_VENV/bin/rnsd" ] &&
    [ -x "$RNS_VENV/bin/rnstatus" ] && [ -x "$RNS_VENV/bin/rnodeconf" ] &&
    "$RNS_VENV/bin/python" -I -c 'import RNS, LXMF' >/dev/null 2>&1; then
    return 0
  fi
  rm -rf "$RNS_VENV" "$RNS_WHEELS" "$dir/lock.sha256"
  mkdir -p "$RNS_WHEELS"
  if python3 -m venv "$RNS_VENV" >"$dir/pip.log" 2>&1 &&
    "$RNS_VENV/bin/pip" download -q --require-hashes --only-binary=:all: -r "$RNS_LOCK" -d "$RNS_WHEELS" >>"$dir/pip.log" 2>&1 &&
    "$RNS_VENV/bin/pip" install -q --require-hashes --no-index --find-links "$RNS_WHEELS" -r "$RNS_LOCK" >>"$dir/pip.log" 2>&1 &&
    "$RNS_VENV/bin/python" -I -c 'import RNS, LXMF' >>"$dir/pip.log" 2>&1; then
    printf '%s\n' "$RNS_LOCKSHA" >"$dir/lock.sha256"
    return 0
  fi
  RNS_VENV_ERR="cannot build the venv from the lock: $(grep -m1 -E 'ERROR|error' "$dir/pip.log" 2>/dev/null)"
  rm -rf "$RNS_VENV" "$RNS_WHEELS"
  return 1
}

rns_venv_remove() {
  local dir=${1:-${RNS_VENV_DIR:-${TMPDIR:-/tmp}/111-rns-venv}}
  rm -rf "$dir/venv" "$dir/wheels" "$dir/lock.sha256" "$dir/pip.log"
  rmdir "$dir" 2>/dev/null || true
}

if [ "${BASH_SOURCE[0]}" = "$0" ]; then
  set -u
  if rns_venv_ensure "${1:-}"; then
    printf '%s\n' "$RNS_VENV"
  else
    printf 'rns-venv: %s\n' "$RNS_VENV_ERR" >&2
    exit 1
  fi
fi
