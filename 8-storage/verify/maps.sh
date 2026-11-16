#!/usr/bin/env bash
# 8-storage/verify/maps.sh — Layer 8 gate G17 (spec §18 G17; §10):
#   maps: the tiles routes over a PMTiles fixture gen-pmtiles.py writes into the scratch root, and the UI checks.
#
#   bash 8-storage/verify/maps.sh                  runs when its prerequisites are present, else SKIPPED
#   REQUIRE_G17=1 bash 8-storage/verify/maps.sh    lock run: a missing prerequisite is a FAIL
#
# T0 stub (8-storage/TASKS.md "T0 — lead scaffold"): the assertions land with row V7.
# Starts nothing, so nothing of ours stays running.
set -u
cd "$(dirname "$0")/../.." || exit 1
# shellcheck source=8-storage/verify/lib.sh
. 8-storage/verify/lib.sh
skip_or_fail G17 "not built yet (T0 stub)"
