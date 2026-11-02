#!/usr/bin/env bash
# 7-relay/verify/forward.sh — Layer 7 gate G9 (spec §15 G9; §5):
#   cross-hub custody forward; hubs A, B, C in netns, A and B on TEST-NET so TLS applies.
#
#   bash 7-relay/verify/forward.sh              runs when its prerequisites are present, else SKIPPED
#   REQUIRE_G9=1 bash 7-relay/verify/forward.sh    lock run: a missing prerequisite is a FAIL
#
# T0 stub (7-relay/TASKS.md "T0 — lead scaffold"): the assertions land with row V3.
# Starts nothing, so nothing of ours stays running.
set -u
cd "$(dirname "$0")/../.." || exit 1
# shellcheck source=7-relay/verify/lib.sh
. 7-relay/verify/lib.sh
skip_or_fail G9 "not built yet (T0 stub)"
