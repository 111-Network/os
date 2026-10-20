#!/data/data/com.termux/files/usr/bin/sh
# 111 node — Termux:Boot entry point.
# Install path on device: ~/.termux/boot/00-111-node.sh
# Runs at boot (on lockscreen phones: at first unlock — FBE/BFU reality, spec §6).
# Order per spec §Supervision Android: termux-wake-lock → profile → runsvdir.
# Idempotent: safe to run any number of times (boot + provisioner re-runs).

set -u

PREFIX=${PREFIX:-/data/data/com.termux/files/usr}
HOME=${HOME:-/data/data/com.termux/files/home}
export PREFIX HOME

LOG="$PREFIX/var/log/111-boot.log"
mkdir -p "$PREFIX/var/log" "$PREFIX/var/lib/111/state"

say() { printf '%s 111-boot: %s\n' "$(date -Iseconds 2>/dev/null || date)" "$*" >> "$LOG"; }

say "boot entry (pid $$)"

# 1. keep the device awake for us (Termux:API)
if command -v termux-wake-lock >/dev/null 2>&1; then
	if termux-wake-lock; then
		say "wake-lock held"
	else
		say "WARNING: termux-wake-lock failed"
	fi
else
	say "WARNING: termux-wake-lock missing (Termux:API not installed?)"
fi

# 2. node environment
# shellcheck disable=SC1091  # path is $PREFIX-relative on device; intentionally dynamic
if [ -f "$PREFIX/var/lib/111/profile.env" ]; then
	# shellcheck disable=SC1091  # same dynamic-path reason
	. "$PREFIX/var/lib/111/profile.env"
	say "profile loaded"
else
	say "WARNING: profile.env missing — node not provisioned yet; exiting cleanly"
	exit 0
fi

# 3. runit supervision
if pgrep -f runsvdir >/dev/null 2>&1; then
	say "runsvdir already running"
else
	# detached: nohup + </dev/null so an adb/activity teardown can't SIGHUP it
	nohup runsvdir "$PREFIX/var/service" </dev/null >>"$LOG" 2>&1 &
	say "runsvdir started"
fi

# give runsvdir a moment to scan, then ensure the node service is up
_i=0
while [ "$_i" -lt 15 ]; do
	if sv status "$PREFIX/var/service/111node" >/dev/null 2>&1; then
		break
	fi
	_i=$((_i + 1))
	sleep 1
done
if sv start "$PREFIX/var/service/111node" >/dev/null 2>&1; then
	say "111node started"
else
	say "WARNING: sv start 111node failed"
fi

say "boot done"
exit 0
