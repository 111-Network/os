#!/usr/bin/env bash
# 7-relay/verify/presence.sh — Layer 7 gate G8 (spec §15 G8; §2, §7.1, §7.7):
#   presence, the Nearby `111` rows and the registry; hub H, members M1 and M2 in netns.
#
#   bash 7-relay/verify/presence.sh              runs when its prerequisites are present, else SKIPPED
#   REQUIRE_G8=1 bash 7-relay/verify/presence.sh    lock run: a missing prerequisite is a FAIL
#
# T0 stub (7-relay/TASKS.md "T0 — lead scaffold"): the assertions land with row V2.
# Starts nothing, so nothing of ours stays running.
set -u
cd "$(dirname "$0")/../.." || exit 1
# shellcheck source=7-relay/verify/lib.sh
. 7-relay/verify/lib.sh
skip_or_fail G8 "not built yet (T0 stub)"
