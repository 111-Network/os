#!/usr/bin/env bash
# 7-relay/verify/idle.sh — Layer 7 gate G7 (spec §15 G7 extended; §3.7):
#   the no-timer gate: wakeups and outbound of an idle 111d in separate windows.
#
#   bash 7-relay/verify/idle.sh              runs when its prerequisites are present, else SKIPPED
#   REQUIRE_G7=1 bash 7-relay/verify/idle.sh    lock run: a missing prerequisite is a FAIL
#
# T0 stub (7-relay/TASKS.md "T0 — lead scaffold"): the assertions land with row V1 (V7 fixes).
# Starts nothing, so nothing of ours stays running.
set -u
cd "$(dirname "$0")/../.." || exit 1
# shellcheck source=7-relay/verify/lib.sh
. 7-relay/verify/lib.sh
skip_or_fail G7 "not built yet (T0 stub)"
