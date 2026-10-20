#!/data/data/com.termux/files/usr/bin/sh
# 111 E2 recovery stub — LAST-RESORT provisioning channel (spec phone step 4).
#
# Used only when the `run-as com.termux` channel fails. The provisioner types
# this script into an open Termux session via chunked `input text` (each chunk
# ≤3800 bytes of a base64 form of this file, one `printf` append per chunk),
# keeping the screen awake; after each chunk it types a probe line and reads the
# terminal via `uiautomator dump` to compare the echoed sha256 of what landed.
# Once the full stub verifies, it is executed and pulls the REAL payload over
# `adb reverse` (phone 127.0.0.1:@@E2_PORT@@ → host HTTP server).
#
# This file must stay ≤4KB total. Tokens the provisioner substitutes:
#   @@E2_PORT@@         host port forwarded via adb reverse
#   @@PAYLOAD_SHA256@@  expected sha256 of the payload archive
#   @@DEST@@            where the payload lands (e.g. $PREFIX/tmp/111-payload.tgz)
#
# Size guard (checked by termux/tests/dryrun.sh): wc -c <= 4096.

set -eu

PREFIX=${PREFIX:-/data/data/com.termux/files/usr}
DEST="@@DEST@@"
WANT="@@PAYLOAD_SHA256@@"
URL="http://127.0.0.1:@@E2_PORT@@/payload"

printf 'e2e-stub: alive\n'          # host reads this via uiautomator dump
mkdir -p "$PREFIX/tmp"
rm -f "$DEST"

if command -v curl >/dev/null 2>&1; then
	curl -fsS -o "$DEST" "$URL"
elif command -v wget >/dev/null 2>&1; then
	wget -qO "$DEST" "$URL"
else
	printf 'e2e-stub: FAIL no curl/wget\n'
	exit 1
fi

GOT=$(sha256sum "$DEST" | cut -d' ' -f1)
printf 'e2e-stub: sha256:%s\n' "$GOT"   # echoed for host-side comparison
[ "$GOT" = "$WANT" ] || {
	printf 'e2e-stub: FAIL sha256 mismatch (want %s)\n' "$WANT"
	rm -f "$DEST"
	exit 1
}

printf 'e2e-stub: payload verified, handing off\n'
chmod 0755 "$DEST" 2>/dev/null || true
exec sh "$DEST"
