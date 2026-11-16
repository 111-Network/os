#!/usr/bin/env bash
# 8-storage/verify/budget.sh — Layer 8 gate G14 (spec §18 G14; §4, §12.3, §13.4, §13.8):
#   the budget, the log rings, no idle write, the consumer registry; one node whose root is a small tmpfs.
#
#   bash 8-storage/verify/budget.sh                  runs when its prerequisites are present, else SKIPPED
#   REQUIRE_G14=1 bash 8-storage/verify/budget.sh    lock run: a missing prerequisite is a FAIL
#
# T0 stub (8-storage/TASKS.md "T0 — lead scaffold"): the assertions land with row V4.
# Starts nothing, so nothing of ours stays running.
set -u
cd "$(dirname "$0")/../.." || exit 1
# shellcheck source=8-storage/verify/lib.sh
. 8-storage/verify/lib.sh
skip_or_fail G14 "not built yet (T0 stub)"
