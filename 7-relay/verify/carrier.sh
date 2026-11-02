#!/usr/bin/env bash
# 7-relay/verify/carrier.sh — Layer 7 gate G10 (spec §15 G10; §5.9, §6.1–§6.4):
#   the ntfy carrier; gateways G and G′ joined only through stub-ntfy.py (P1 adds --smoke).
#
#   bash 7-relay/verify/carrier.sh              runs when its prerequisites are present, else SKIPPED
#   REQUIRE_G10=1 bash 7-relay/verify/carrier.sh    lock run: a missing prerequisite is a FAIL
#
# T0 stub (7-relay/TASKS.md "T0 — lead scaffold"): the assertions land with row P1 (smoke mode) and V4.
# Starts nothing, so nothing of ours stays running.
set -u
cd "$(dirname "$0")/../.." || exit 1
# shellcheck source=7-relay/verify/lib.sh
. 7-relay/verify/lib.sh
skip_or_fail G10 "not built yet (T0 stub)"
