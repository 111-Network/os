#!/usr/bin/env bash
# 9-evolution/verify/lib.sh — shared names for the Layer 9 gate scripts (spec §20 "Gate homes").
#
# Sourced, never run. It sources 6-transport/verify/lib.sh (ok/note/fail/skip_or_fail/have_netns/finish and the
# REQUIRE_<GATE> + SKIPPED convention) and 6-transport/verify/root.sh (the scratch NODE111_ROOT helpers), then names
# the netns, veth, address and port names the §20 gates use, so G19–G25 never invent their own. Names only: sourcing
# this file creates no namespace, link, mount, process or file. Every harness stops what it started (its netns, its
# 111d's, the stub model, the stub child and any opencode), leaves nothing listening and runs alone, never beside a
# 6-transport/verify/, 7-relay/verify/ or 8-storage/verify/ harness (AGENTS.md, Linux hygiene; 9-evolution/TASKS.md
# box rules).
#
# Addresses: one private /24 per gate, 10.111.91.0/24 (G19) to 10.111.97.0/24 (G25), §20's 10.111.9x.0/24 (RFC 1918;
# clear of Layer 7's 10.111.7x, 10.111.80 and 10.111.89 blocks and Layer 8's 10.111.83 to 10.111.88). The values are
# T0 defaults; the row that fills a gate (V3–V9) may change its own block.
# shellcheck disable=SC2034
L9_VERIFY=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
L9_OS=$(cd "$L9_VERIFY/../.." && pwd)
# shellcheck source=6-transport/verify/lib.sh
. "$L9_OS/6-transport/verify/lib.sh"
# shellcheck source=6-transport/verify/root.sh
. "$L9_OS/6-transport/verify/root.sh"

L9_LOGS=$L9_VERIFY/logs       # harness logs (the 7-relay/verify/logs/ precedent)
L9_TAPES=$L9_VERIFY/tapes     # recorded tapes: applier-l6/ (row P1, G20 (1)), the Layer 8 frames (row P2, G22 (6)),
#                               model/ (row V2: the stub model's recorded completions)
L9_VECTORS=$L9_VERIFY/vectors # known-answer vectors: fix-v1.json (row D7, G22 (1))

# ---- ports (the daemon's and its children's own; the stub model's own is ours to pick)
L9_PORT_DASH=11180       # dashboard + /api/v1 (N/dashboard/mod.rs DEFAULT_PORT): /api/v1/repairs*, the decision route;
#                          G24 (2): no connection to it from opencode
L9_PORT_MODEL=11181      # the model child (N/intelligence.rs MODEL_PORT): stub-model.py sitting at current/bin/llama-server
#                          serves here (G21; G23 on H)
L9_PORT_AGENT=11182      # the real 111-agent (N/intelligence.rs AGENT_PORT): the loopback task API T1 uses (G21, G23-G25)
L9_PORT_FABRIC=11183     # tcp-lan fabric listener: fixes and delegations between nodes (G22, G23)
L9_PORT_STUB_MODEL=11480 # stub-model.py when a harness starts it itself rather than as the model child (row V2's pick)

# ---- G19 the observer (§20 G19): one node with the real 111d, a real supervised stub child and the real agent off;
# loopback plus a veth with no default route (7-relay/verify/idle.sh's shape: the G7 phase, row V10, starts its own idle
# 111d here and compares against it)
L9_G19_VETH=l9g19v
L9_G19_VETH_PEER=l9g19p
L9_G19_NET=10.111.91.0/24
L9_G19_IP=10.111.91.11
L9_G19_PEER_IP=10.111.91.1

# ---- G20 the engine (§20 G20): one node with G1's bridge stubs (NODE111_BRIDGE_STUBS, debug builds) and G5's device
# fakes (6-transport/verify/devices.sh); a veth to one LAN client for the off-loopback requests (item (4): a confirm
# from another session; item (9): an accept from a non-loopback session, 403 loopback_only)
L9_G20_VETH=l9g20v
L9_G20_VETH_PEER=l9g20p
L9_G20_NET=10.111.92.0/24
L9_G20_IP=10.111.92.11
L9_G20_PEER_IP=10.111.92.1

# ---- G21 diagnosis (§20 G21): one node with the real agent and stub-model.py as the model child; loopback plus a veth
# with no default route, so a request that leaves the node shows in the netns socket log (item (11))
L9_G21_VETH=l9g21v
L9_G21_VETH_PEER=l9g21p
L9_G21_NET=10.111.93.0/24
L9_G21_IP=10.111.93.11
L9_G21_PEER_IP=10.111.93.1

# ---- G22 sharing fixes (§20 G22): nodes A and B and hub H on one bridge over tcp-lan; item (11) runs on Layer 7's G9
# topology, whose names are 7-relay/verify/lib.sh's L7_G9_* (row V6 builds it through this file)
L9_G22_BR=l9g22br0
L9_G22_NET=10.111.94.0/24
L9_G22_NODES="H A B"
L9_G22_IP_H=10.111.94.10
L9_G22_IP_A=10.111.94.11
L9_G22_IP_B=10.111.94.12
L9_G22_VETH_H=l9g22h
L9_G22_VETH_A=l9g22a
L9_G22_VETH_B=l9g22b

# ---- G23 delegation (§20 G23): hub H and members M1, M2 on one bridge over tcp-lan; stub-model.py as H's model child
L9_G23_BR=l9g23br0
L9_G23_NET=10.111.95.0/24
L9_G23_NODES="H M1 M2"
L9_G23_IP_H=10.111.95.10
L9_G23_IP_M1=10.111.95.11
L9_G23_IP_M2=10.111.95.12
L9_G23_VETH_H=l9g23h
L9_G23_VETH_M1=l9g23m1
L9_G23_VETH_M2=l9g23m2

# ---- G24 code drafts (§20 G24): one hub with the real agent, the stub model and OPENCODE_BIN (a pinned opencode,
# fetched into the scratchpad only, never vendored); loopback plus a veth with no default route, so a connection
# opencode makes shows in the netns socket log (item (2))
L9_G24_VETH=l9g24v
L9_G24_VETH_PEER=l9g24p
L9_G24_NET=10.111.96.0/24
L9_G24_IP=10.111.96.11
L9_G24_PEER_IP=10.111.96.1

# ---- G25 evolution (§20 G25): build.py fixtures and agent111 tests; items (2) and (4) start one node with the real
# agent and the stub model (a tampered share/agent/ file; "Draft a KB note" and a kb_draft accepted)
L9_G25_VETH=l9g25v
L9_G25_VETH_PEER=l9g25p
L9_G25_NET=10.111.97.0/24
L9_G25_IP=10.111.97.11
L9_G25_PEER_IP=10.111.97.1
