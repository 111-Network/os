#!/usr/bin/env bash
# 9-evolution/verify/engine.sh — Layer 9 gate G20 (spec §20 G20; §7, §16 rows 15 and 18, §18):
#   the engine; G1's bridge stubs (NODE111_BRIDGE_STUBS, debug builds), G5's device fakes and the applier tape
#   (9-evolution/verify/tapes/applier-l6/); node111 and UI tests.
#
#   bash 9-evolution/verify/engine.sh                  runs when its prerequisites are present, else SKIPPED
#   REQUIRE_G20=1 bash 9-evolution/verify/engine.sh    lock run: a missing prerequisite is a FAIL
#
# T0 stub (9-evolution/TASKS.md "T0 — lead scaffold"): the assertions land with row V4.
# Starts nothing, so nothing of ours stays running.
set -u
cd "$(dirname "$0")/../.." || exit 1
# shellcheck source=9-evolution/verify/lib.sh
. 9-evolution/verify/lib.sh
skip_or_fail G20 "not built yet (T0 stub)"
