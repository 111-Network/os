#!/usr/bin/env bash
# 9-evolution/verify/observer.sh — Layer 9 gate G19 (spec §20 G19; §3, §4.1, §4.2, §12.6), and the G7 extended phase
# (§20 "G7, extended"): the observer; one netns node with the real 111d, a real supervised stub child and the real
# agent off; node111 tests.
#
#   bash 9-evolution/verify/observer.sh                  runs when its prerequisites are present, else SKIPPED
#   REQUIRE_G19=1 bash 9-evolution/verify/observer.sh    lock run: a missing prerequisite is a FAIL
#
# T0 stub (9-evolution/TASKS.md "T0 — lead scaffold"): the assertions land with row V3, the G7 phase with row V10.
# Starts nothing, so nothing of ours stays running.
set -u
cd "$(dirname "$0")/../.." || exit 1
# shellcheck source=9-evolution/verify/lib.sh
. 9-evolution/verify/lib.sh
skip_or_fail G19 "not built yet (T0 stub)"
