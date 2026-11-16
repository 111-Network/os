#!/usr/bin/env bash
# 9-evolution/verify/delegate.sh — Layer 9 gate G23 (spec §20 G23; §12):
#   delegation; hub H and members M1, M2 in netns over tcp-lan, stub-model.py on H; mesh111 tests.
#
#   bash 9-evolution/verify/delegate.sh                  runs when its prerequisites are present, else SKIPPED
#   REQUIRE_G23=1 bash 9-evolution/verify/delegate.sh    lock run: a missing prerequisite is a FAIL
#
# T0 stub (9-evolution/TASKS.md "T0 — lead scaffold"): the assertions land with row V7.
# Starts nothing, so nothing of ours stays running.
set -u
cd "$(dirname "$0")/../.." || exit 1
# shellcheck source=9-evolution/verify/lib.sh
. 9-evolution/verify/lib.sh
skip_or_fail G23 "not built yet (T0 stub)"
