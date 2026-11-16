#!/usr/bin/env bash
# 8-storage/verify/components.sh — Layer 8 gate G18 (spec §18 G18; §11):
#   component pins from a hub: origin.components, the installer's origin list, the files table, the source tarball.
#
#   bash 8-storage/verify/components.sh                  runs when its prerequisites are present, else SKIPPED
#   REQUIRE_G18=1 bash 8-storage/verify/components.sh    lock run: a missing prerequisite is a FAIL
#
# T0 stub (8-storage/TASKS.md "T0 — lead scaffold"): the assertions land with row V8.
# Starts nothing, so nothing of ours stays running.
set -u
cd "$(dirname "$0")/../.." || exit 1
# shellcheck source=8-storage/verify/lib.sh
. 8-storage/verify/lib.sh
skip_or_fail G18 "not built yet (T0 stub)"
