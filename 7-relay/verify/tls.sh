#!/usr/bin/env bash
# 7-relay/verify/tls.sh — Layer 7 gate G11 (spec §15 G11; §4.1–§4.5):
#   TLS 1.3 raw public keys for paired-hub links; two hubs on TEST-NET in netns.
#
#   bash 7-relay/verify/tls.sh              runs when its prerequisites are present, else SKIPPED
#   REQUIRE_G11=1 bash 7-relay/verify/tls.sh    lock run: a missing prerequisite is a FAIL
#
# T0 stub (7-relay/TASKS.md "T0 — lead scaffold"): the assertions land with row V5.
# Starts nothing, so nothing of ours stays running.
set -u
cd "$(dirname "$0")/../.." || exit 1
# shellcheck source=7-relay/verify/lib.sh
. 7-relay/verify/lib.sh
skip_or_fail G11 "not built yet (T0 stub)"
