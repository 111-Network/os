#!/usr/bin/env bash
# 8-storage/verify/state.sh — Layer 8 gate G13 (spec §18 G13; §2, §3, §13.2, §13.6, §13.7):
#   the node-state store and the joint contract; one node with the real 111d, the real 111-agent and a stub model.
#
#   bash 8-storage/verify/state.sh                  runs when its prerequisites are present, else SKIPPED
#   REQUIRE_G13=1 bash 8-storage/verify/state.sh    lock run: a missing prerequisite is a FAIL
#
# T0 stub (8-storage/TASKS.md "T0 — lead scaffold"): the assertions land with row V3.
# Starts nothing, so nothing of ours stays running.
set -u
cd "$(dirname "$0")/../.." || exit 1
# shellcheck source=8-storage/verify/lib.sh
. 8-storage/verify/lib.sh
skip_or_fail G13 "not built yet (T0 stub)"
