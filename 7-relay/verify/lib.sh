#!/usr/bin/env bash
# 7-relay/verify/lib.sh — shared names for the Layer 7 gate scripts (spec §15 "Gate homes").
#
# Sourced, never run. It sources 6-transport/verify/lib.sh (ok/note/fail/skip_or_fail/have_netns/finish and the
# REQUIRE_<GATE> + SKIPPED convention) and 6-transport/verify/root.sh (the scratch NODE111_ROOT helpers), then names
# the netns, veth, address and port names the §15 gates use, so G7–G12 never invent their own. Names only: sourcing
# this file creates no namespace, link, process or file. Every harness stops what it started (AGENTS.md, Linux
# hygiene) and runs one at a time (7-relay/TASKS.md box rules).
#
# Addresses: private LANs in 10.111.7x.0/24 (RFC 1918, so a pair there is private and runs plain, §4.1); the
# "internet" hubs on the RFC 5737 TEST-NET ranges, which §4.1 does not count as private, so TLS-RPK applies to them.
# The values are T0 defaults; the row that fills a gate may change its own block.
# shellcheck disable=SC2034
L7_VERIFY=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
L7_OS=$(cd "$L7_VERIFY/../.." && pwd)
# shellcheck source=6-transport/verify/lib.sh
. "$L7_OS/6-transport/verify/lib.sh"
# shellcheck source=6-transport/verify/root.sh
. "$L7_OS/6-transport/verify/root.sh"

L7_TAPES=$L7_VERIFY/tapes # committed frame tapes (G9 item 1, the 6-transport/verify/tapes/ precedent)
L7_LOGS=$L7_VERIFY/logs   # smoke logs (§6.4: logs/ntfy-smoke-<calver>.md, P1's -stub.md)

# ---- ports (the daemon's own; the stub ntfy server's is ours to pick)
L7_PORT_DASH=11180   # dashboard + /api/v1 + /origin/v1 (N/dashboard/mod.rs DEFAULT_PORT; §9.1, S4)
L7_PORT_FABRIC=11183 # tcp-lan fabric listener: members, paired hubs, TLS-RPK peek (§4.1 rule 2, §7.1 SRV)
L7_PORT_NTFY=11280   # stub-ntfy.py (§15 G10) or an NTFY_BIN server inside its netns

# ---- G8 presence and the registry (§15 G8): hub H, members M1 and M2 on one bridge, M1 and M2 also direct
L7_G8_BR=l7g8br0
L7_G8_NET=10.111.78.0/24
L7_G8_NODES="H M1 M2"
L7_G8_IP_H=10.111.78.10
L7_G8_IP_M1=10.111.78.11
L7_G8_IP_M2=10.111.78.12
L7_G8_VETH_H=l7g8h
L7_G8_VETH_M1=l7g8m1
L7_G8_VETH_M2=l7g8m2

# ---- G9 cross-hub forward (§15 G9): hubs A, B, C; A and B on TEST-NET-1 so TLS applies; member M on A, D on B
L7_G9_WAN_BR=l7g9wan
L7_G9_WAN_NET=192.0.2.0/24 # TEST-NET-1 (RFC 5737)
L7_G9_NODES="A B C M D"
L7_G9_IP_A=192.0.2.11
L7_G9_IP_B=192.0.2.12
L7_G9_IP_C=192.0.2.13
L7_G9_LAN_A=10.111.79.0/24 # A's LAN: A .1, M .11
L7_G9_LAN_A_IP_A=10.111.79.1
L7_G9_IP_M=10.111.79.11
L7_G9_LAN_B=10.111.89.0/24 # B's LAN: B .1, D .11
L7_G9_LAN_B_IP_B=10.111.89.1
L7_G9_IP_D=10.111.89.11
L7_G9_VETH_A=l7g9a
L7_G9_VETH_B=l7g9b
L7_G9_VETH_C=l7g9c
L7_G9_VETH_M=l7g9m
L7_G9_VETH_D=l7g9d
L7_G9_OLD_TAG=checkpoint-layer-6 # the old hub B of item 6 (4689dbf), exported under the scratchpad

# ---- G10 the carrier (§15 G10): gateways G and G' in two netns, each with a leaf hub, joined only through the stub
L7_G10_WAN_BR=l7g10wan
L7_G10_WAN_NET=198.51.100.0/24 # TEST-NET-2 (RFC 5737)
L7_G10_NODES="G GP LG LGP NTFY"
L7_G10_IP_NTFY=198.51.100.1
L7_G10_IP_G=198.51.100.11
L7_G10_IP_GP=198.51.100.12 # G' (G prime)
L7_G10_LAN_G=10.111.70.0/24 # G and its leaf hub LG
L7_G10_IP_LG=10.111.70.11
L7_G10_LAN_GP=10.111.80.0/24 # G' and its leaf hub LGP
L7_G10_IP_LGP=10.111.80.11
L7_G10_VETH_G=l7g10g
L7_G10_VETH_GP=l7g10gp
L7_G10_VETH_LG=l7g10lg
L7_G10_VETH_LGP=l7g10lgp
L7_G10_VETH_NTFY=l7g10n

# ---- G11 TLS-RPK (§15 G11): two hubs on TEST-NET-3
L7_G11_BR=l7g11br0
L7_G11_NET=203.0.113.0/24 # TEST-NET-3 (RFC 5737)
L7_G11_NODES="X Y"
L7_G11_IP_X=203.0.113.11
L7_G11_IP_Y=203.0.113.12
L7_G11_VETH_X=l7g11x
L7_G11_VETH_Y=l7g11y

# ---- G12 the hub origin (§15 G12): hub O with LAN sharing, member U with no internet route
L7_G12_BR=l7g12br0
L7_G12_NET=10.111.72.0/24
L7_G12_NODES="O U"
L7_G12_IP_O=10.111.72.10
L7_G12_IP_U=10.111.72.11
L7_G12_VETH_O=l7g12o
L7_G12_VETH_U=l7g12u

# ---- G7 extended, idle.sh (§3.7): one fresh netns, loopback plus a veth with no default route
L7_G7_VETH=l7g7v
L7_G7_VETH_PEER=l7g7p
L7_G7_IP=10.111.77.11
L7_G7_PEER_IP=10.111.77.1
L7_G7_NET=10.111.77.0/24
