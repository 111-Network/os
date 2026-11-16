#!/usr/bin/env bash
# 8-storage/verify/lib.sh — shared names for the Layer 8 gate scripts (spec §18 "Gate homes").
#
# Sourced, never run. It sources 6-transport/verify/lib.sh (ok/note/fail/skip_or_fail/have_netns/finish and the
# REQUIRE_<GATE> + SKIPPED convention) and 6-transport/verify/root.sh (the scratch NODE111_ROOT helpers), then names
# the netns, veth, address and port names the §18 gates use, so G13–G18 never invent their own. Names only: sourcing
# this file creates no namespace, link, mount, process or file. Every harness stops what it started, unmounts the tmpfs
# it mounted (G14's root, G15 (11)'s second filesystem) and runs alone, never beside a 6-transport/verify/ or
# 7-relay/verify/ harness (AGENTS.md, Linux hygiene; 8-storage/TASKS.md box rules).
#
# Addresses: one private /24 per gate, 10.111.83.0/24 (G13) to 10.111.88.0/24 (G18) (RFC 1918; clear of Layer 7's
# 10.111.7x, 10.111.80 and 10.111.89 blocks). The values are T0 defaults; the row that fills a gate (V3–V8) may change
# its own block.
# shellcheck disable=SC2034
L8_VERIFY=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
L8_OS=$(cd "$L8_VERIFY/../.." && pwd)
# shellcheck source=6-transport/verify/lib.sh
. "$L8_OS/6-transport/verify/lib.sh"
# shellcheck source=6-transport/verify/root.sh
. "$L8_OS/6-transport/verify/root.sh"

L8_LOGS=$L8_VERIFY/logs # harness logs (the 7-relay/verify/logs/ precedent)

# ---- ports (the daemon's and its children's own; the stub mirror's is ours to pick)
L8_PORT_DASH=11180     # dashboard + /api/v1 + /origin/v1 (N/dashboard/mod.rs DEFAULT_PORT; §16.7)
L8_PORT_MODEL=11181    # the stub model, spawned as llama-server by the daemon (N/intelligence.rs MODEL_PORT; G13)
L8_PORT_AGENT=11182    # the real 111-agent (N/intelligence.rs AGENT_PORT; G13)
L8_PORT_FABRIC=11183   # tcp-lan fabric listener: a member and its home hub (§6.2, §11.3; G15, G18)
L8_PORT_PANEL_LO=11190 # the panel block 11190-11229 (N/panels/mod.rs PANEL_PORTS): G16's kiwix panel listener
L8_PORT_PANEL_HI=11229
L8_PORT_CHILD=11250    # the kiwix component's declared loopback port (§9.2): kiwix-serve (G16 part B) or stub-child.py
#                        (part A), which serves on the declared port (§18 "Gate homes"; G16: 127.0.0.1:11250 only)
L8_PORT_MIRROR=11380   # stub-mirror.py (§18 "Gate homes"; G15)

# ---- G13 the node-state store (§18 G13): one node with the real 111d, the real 111-agent and a stub model; a veth
# to one LAN client for the off-loopback requests (a paired session off loopback, 403 loopback_only)
L8_G13_VETH=l8g13v
L8_G13_VETH_PEER=l8g13p
L8_G13_NET=10.111.83.0/24
L8_G13_IP=10.111.83.11
L8_G13_PEER_IP=10.111.83.1

# ---- G14 the budget (§18 G14): one node whose root is a small tmpfs mounted inside the user namespace (its size is
# the filling row's); a veth to one LAN client
L8_G14_VETH=l8g14v
L8_G14_VETH_PEER=l8g14p
L8_G14_NET=10.111.84.0/24
L8_G14_IP=10.111.84.11
L8_G14_PEER_IP=10.111.84.1

# ---- G15 datasets (§18 G15): hubs H and H2 and a member M on one bridge, and the stub mirror on the same bridge
L8_G15_BR=l8g15br0
L8_G15_NET=10.111.85.0/24
L8_G15_NODES="H H2 M MIRROR"
L8_G15_IP_H=10.111.85.10
L8_G15_IP_H2=10.111.85.12
L8_G15_IP_M=10.111.85.11
L8_G15_IP_MIRROR=10.111.85.20
L8_G15_VETH_H=l8g15h
L8_G15_VETH_H2=l8g15h2
L8_G15_VETH_M=l8g15m
L8_G15_VETH_MIRROR=l8g15s

# ---- G16 the encyclopedia (§18 G16): one node with the stub child (part A) or kiwix-serve (part B) on loopback; a
# veth to one LAN client for the panel listener
L8_G16_VETH=l8g16v
L8_G16_VETH_PEER=l8g16p
L8_G16_NET=10.111.86.0/24
L8_G16_IP=10.111.86.11
L8_G16_PEER_IP=10.111.86.1

# ---- G17 maps (§18 G17): one node; a veth to one LAN client (a request with no session off loopback is refused)
L8_G17_VETH=l8g17v
L8_G17_VETH_PEER=l8g17p
L8_G17_NET=10.111.87.0/24
L8_G17_IP=10.111.87.11
L8_G17_PEER_IP=10.111.87.1

# ---- G18 component pins from a hub (§18 G18): hub H and a member or provisioning host M on one bridge, M with no
# internet route
L8_G18_BR=l8g18br0
L8_G18_NET=10.111.88.0/24
L8_G18_NODES="H M"
L8_G18_IP_H=10.111.88.10
L8_G18_IP_M=10.111.88.11
L8_G18_VETH_H=l8g18h
L8_G18_VETH_M=l8g18m
