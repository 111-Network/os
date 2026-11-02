#!/usr/bin/env bash
# 6-transport/verify/root.sh — shared helpers for the Layer 6 gate scripts that run a real
# `111 daemon` (spec §16): a scratch NODE111_ROOT the way 4-interface/verify/smoke.sh builds one
# (identity, releases/<ver>, current → it, etc/env, state/), trimmed to what a bridge gate needs.
#
# Sourced, never run. Functions (all take explicit paths; nothing global but G6_VER):
#   g6_make_root ROOT ENVTEXT   releases/$G6_VER/bin/111-agent = a stand-in that only records
#                               the daemon's per-spawn bearer (Layer 5 §17) into
#                               <root>/agent/g6-agent.token (0600) and sleeps, so a gate can
#                               present the agent's real bearer; no model, no KB (the daemon
#                               logs "KB integrity" and serves on: /health may answer 503)
#   g6_identity ROOT BIN111     `111 identity generate` → prints the 64-hex node id (or nothing)
#   g6_fabric_settings ROOT JSON  fabric/settings (0700 dir, 0600 file), as smoke.sh's fabset
#
# The stand-in agent is a shell script that execs `sleep`: it dies with the daemon's pid
# namespace or with the daemon (the supervisor's PDEATHSIG), never outlives a gate.
G6_VER=${G6_VER:-2026.10.20}

g6_make_root() {
  local root=$1 envtext=$2
  local rel=$root/releases/$G6_VER
  mkdir -p "$rel/bin" "$root/etc" "$root/state"
  ln -sfn "releases/$G6_VER" "$root/current"
  cat >"$rel/bin/111-agent" <<'EOF'
#!/bin/sh
# Layer 6 gate stand-in for 111-agent: record the daemon's bearer for the 401 matrix, then idle.
umask 077
if [ -n "${AGENT111_STATE_DIR:-}" ] && [ -n "${AGENT111_DAEMON_TOKEN:-}" ]; then
  printf '%s' "$AGENT111_DAEMON_TOKEN" >"$AGENT111_STATE_DIR/g6-agent.token.tmp" &&
    mv "$AGENT111_STATE_DIR/g6-agent.token.tmp" "$AGENT111_STATE_DIR/g6-agent.token"
fi
exec sleep 1000000
EOF
  chmod 755 "$rel/bin/111-agent"
  printf '%s\n' "$envtext" >"$root/etc/env"
}

g6_identity() {
  local root=$1 bin=$2 nid
  nid=$(NODE111_ROOT=$root "$bin" identity generate 2>"$root/identity.log" | tail -1) || true
  if [[ "$nid" =~ ^[0-9a-f]{64}$ ]]; then printf '%s\n' "$nid"; fi
}

g6_fabric_settings() {
  mkdir -p "$1/fabric"
  chmod 700 "$1/fabric"
  printf '%s\n' "$2" >"$1/fabric/settings"
  chmod 600 "$1/fabric/settings"
}
