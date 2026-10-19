#!/usr/bin/env bash
# 60-transport/verify/spine.sh — Layer 6 gate G4 (spec §16): golden renders, real rnsd on the rendered dir, attach/detach/reload, phone relay, migration.
#
#   bash 60-transport/verify/spine.sh          runs when its prerequisites are present, else SKIPPED
#   REQUIRE_G4=1 bash 60-transport/verify/spine.sh   lock run: a missing prerequisite is a FAIL
#
# T0 skeleton (TASKS.md): the assertions land with Wave 3. Nothing of ours stays running.
set -u
cd "$(dirname "$0")/../.." || exit 1
. 60-transport/verify/lib.sh
have_netns || skip_or_fail G4 "unprivileged network namespaces are not available (unshare -Urnm)"
skip_or_fail G4 "gate not implemented yet (Wave 3)"
