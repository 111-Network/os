#!/usr/bin/env bash
# 8-storage/verify/kiwix.sh — Layer 8 gate G16 (spec §18 G16; §9):
#   the encyclopedia: part A the stub child as the kiwix program; part B kiwix-serve (KIWIX_BIN, KIWIX_ZIM).
#
#   bash 8-storage/verify/kiwix.sh                  runs when its prerequisites are present, else SKIPPED
#   REQUIRE_G16=1 bash 8-storage/verify/kiwix.sh    lock run: a missing prerequisite is a FAIL
#
# T0 stub (8-storage/TASKS.md "T0 — lead scaffold"): the assertions land with row V6.
# Starts nothing, so nothing of ours stays running.
set -u
cd "$(dirname "$0")/../.." || exit 1
# shellcheck source=8-storage/verify/lib.sh
. 8-storage/verify/lib.sh
skip_or_fail G16 "not built yet (T0 stub)"
