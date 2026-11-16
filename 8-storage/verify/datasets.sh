#!/usr/bin/env bash
# 8-storage/verify/datasets.sh — Layer 8 gate G15 (spec §18 G15; §5, §6, §7, §11.1):
#   datasets: publish, serve, fetch and import; hubs H and H2 and a member M on one bridge with the stub mirror.
#
#   bash 8-storage/verify/datasets.sh                  runs when its prerequisites are present, else SKIPPED
#   REQUIRE_G15=1 bash 8-storage/verify/datasets.sh    lock run: a missing prerequisite is a FAIL
#
# T0 stub (8-storage/TASKS.md "T0 — lead scaffold"): the assertions land with row V5.
# Starts nothing, so nothing of ours stays running.
set -u
cd "$(dirname "$0")/../.." || exit 1
# shellcheck source=8-storage/verify/lib.sh
. 8-storage/verify/lib.sh
skip_or_fail G15 "not built yet (T0 stub)"
