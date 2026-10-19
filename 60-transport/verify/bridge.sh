#!/usr/bin/env bash
# 60-transport/verify/bridge.sh — Layer 6 gate G1 (spec §16): StubAdapter smoke: one 111d (two on a veth for the loop cases) with three StubAdapters.
#
#   bash 60-transport/verify/bridge.sh          runs when its prerequisites are present, else SKIPPED
#   REQUIRE_G1=1 bash 60-transport/verify/bridge.sh   lock run: a missing prerequisite is a FAIL
#
# T0 skeleton (TASKS.md): the assertions land with Wave 3. Nothing of ours stays running.
set -u
cd "$(dirname "$0")/../.." || exit 1
. 60-transport/verify/lib.sh
have_netns || skip_or_fail G1 "unprivileged network namespaces are not available (unshare -Urnm)"
skip_or_fail G1 "gate not implemented yet (Wave 3)"
