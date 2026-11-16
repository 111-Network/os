#!/usr/bin/env bash
# 9-evolution/verify/evolution.sh — Layer 9 gate G25 (spec §20 G25; §6.1, §6.3, §13):
#   evolution; build.py's playbook lint and fixtures, the agent's data by release, kb_draft; agent111 tests.
#
#   bash 9-evolution/verify/evolution.sh                  runs when its prerequisites are present, else SKIPPED
#   REQUIRE_G25=1 bash 9-evolution/verify/evolution.sh    lock run: a missing prerequisite is a FAIL
#
# T0 stub (9-evolution/TASKS.md "T0 — lead scaffold"): the assertions land with row V9.
# Starts nothing, so nothing of ours stays running.
set -u
cd "$(dirname "$0")/../.." || exit 1
# shellcheck source=9-evolution/verify/lib.sh
. 9-evolution/verify/lib.sh
skip_or_fail G25 "not built yet (T0 stub)"
