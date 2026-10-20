#!/usr/bin/env bash
# 6-transport/verify/lxmf.sh — Layer 6 gate G3 (spec §16): two netns, each a real rnsd and one lxmf-sidecar; AF_PACKET egress log.
#
#   bash 6-transport/verify/lxmf.sh          runs when its prerequisites are present, else SKIPPED
#   REQUIRE_G3=1 bash 6-transport/verify/lxmf.sh   lock run: a missing prerequisite is a FAIL
#
# T0 skeleton (TASKS.md): the assertions land with Wave 3. Nothing of ours stays running.
set -u
cd "$(dirname "$0")/../.." || exit 1
. 6-transport/verify/lib.sh
have_netns || skip_or_fail G3 "unprivileged network namespaces are not available (unshare -Urnm)"
skip_or_fail G3 "gate not implemented yet (Wave 3)"
