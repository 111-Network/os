#!/usr/bin/env bash
# 9-evolution/verify/diagnose.sh — Layer 9 gate G21 (spec §20 G21; §5, §6):
#   diagnosis; one node with the real agent and stub-model.py as the model child; node111 and agent111 tests.
#
#   bash 9-evolution/verify/diagnose.sh                  runs when its prerequisites are present, else SKIPPED
#   REQUIRE_G21=1 bash 9-evolution/verify/diagnose.sh    lock run: a missing prerequisite is a FAIL
#
# T0 stub (9-evolution/TASKS.md "T0 — lead scaffold"): the assertions land with row V5.
# Starts nothing, so nothing of ours stays running.
set -u
cd "$(dirname "$0")/../.." || exit 1
# shellcheck source=9-evolution/verify/lib.sh
. 9-evolution/verify/lib.sh
skip_or_fail G21 "not built yet (T0 stub)"
