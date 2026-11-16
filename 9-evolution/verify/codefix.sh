#!/usr/bin/env bash
# 9-evolution/verify/codefix.sh — Layer 9 gate G24 (spec §20 G24; §8, §9):
#   code drafts; OPENCODE_BIN a pinned opencode (fetched by its sha256 into the scratchpad, never vendored), else
#   SKIPPED; the lock run sets REQUIRE_G24=1.
#
#   bash 9-evolution/verify/codefix.sh                  runs when its prerequisites are present, else SKIPPED
#   REQUIRE_G24=1 bash 9-evolution/verify/codefix.sh    lock run: a missing prerequisite is a FAIL
#
# T0 stub (9-evolution/TASKS.md "T0 — lead scaffold"): the assertions land with row V8.
# Starts nothing, so nothing of ours stays running.
set -u
cd "$(dirname "$0")/../.." || exit 1
# shellcheck source=9-evolution/verify/lib.sh
. 9-evolution/verify/lib.sh
skip_or_fail G24 "not built yet (T0 stub)"
