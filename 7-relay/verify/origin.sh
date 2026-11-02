#!/usr/bin/env bash
# 7-relay/verify/origin.sh — Layer 7 gate G12 (spec §15 G12; §9.1–§9.5):
#   the hub as update origin and the Layer 1 trust fixes T1–T6.
#
#   bash 7-relay/verify/origin.sh              runs when its prerequisites are present, else SKIPPED
#   REQUIRE_G12=1 bash 7-relay/verify/origin.sh    lock run: a missing prerequisite is a FAIL
#
# T0 stub (7-relay/TASKS.md "T0 — lead scaffold"): the assertions land with row V6.
# Starts nothing, so nothing of ours stays running.
set -u
cd "$(dirname "$0")/../.." || exit 1
# shellcheck source=7-relay/verify/lib.sh
. 7-relay/verify/lib.sh
skip_or_fail G12 "not built yet (T0 stub)"
