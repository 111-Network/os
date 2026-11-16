#!/usr/bin/env bash
# 9-evolution/verify/share.sh — Layer 9 gate G22 (spec §20 G22; §10, §11):
#   sharing fixes; nodes A and B and hub H in netns over tcp-lan, item (11) on Layer 7's G9 topology; node111 and
#   mesh111 tests.
#
#   bash 9-evolution/verify/share.sh                  runs when its prerequisites are present, else SKIPPED
#   REQUIRE_G22=1 bash 9-evolution/verify/share.sh    lock run: a missing prerequisite is a FAIL
#
# T0 stub (9-evolution/TASKS.md "T0 — lead scaffold"): the assertions land with row V6.
# Starts nothing, so nothing of ours stays running.
set -u
cd "$(dirname "$0")/../.." || exit 1
# shellcheck source=9-evolution/verify/lib.sh
. 9-evolution/verify/lib.sh
skip_or_fail G22 "not built yet (T0 stub)"
