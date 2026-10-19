#!/usr/bin/env bash
# 60-transport/verify/zero-weight.sh — Layer 6 gate G7 (spec §16): processes and listeners equal the Layer 5 baseline with every component disabled; RSS recorded.
#
#   bash 60-transport/verify/zero-weight.sh          runs when its prerequisites are present, else SKIPPED
#   REQUIRE_G7=1 bash 60-transport/verify/zero-weight.sh   lock run: a missing prerequisite is a FAIL
#
# T0 skeleton (TASKS.md): the assertions land with Wave 3. Nothing of ours stays running.
set -u
cd "$(dirname "$0")/../.." || exit 1
. 60-transport/verify/lib.sh
have_netns || skip_or_fail G7 "unprivileged network namespaces are not available (unshare -Urnm)"
skip_or_fail G7 "gate not implemented yet (Wave 3)"
