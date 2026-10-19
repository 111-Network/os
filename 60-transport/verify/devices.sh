#!/usr/bin/env bash
# 60-transport/verify/devices.sh — Layer 6 gate G5 (spec §16): uevent fixtures, PortTable collisions, identify against fakes, the applier.
#
#   bash 60-transport/verify/devices.sh          runs when its prerequisites are present, else SKIPPED
#   REQUIRE_G5=1 bash 60-transport/verify/devices.sh   lock run: a missing prerequisite is a FAIL
#
# T0 skeleton (TASKS.md): the assertions land with Wave 3. Nothing of ours stays running.
set -u
cd "$(dirname "$0")/../.." || exit 1
. 60-transport/verify/lib.sh
have_netns || skip_or_fail G5 "unprivileged network namespaces are not available (unshare -Urnm)"
skip_or_fail G5 "gate not implemented yet (Wave 3)"
