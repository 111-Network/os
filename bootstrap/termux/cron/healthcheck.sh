#!/data/data/com.termux/files/usr/bin/sh
# cronie job: node health probe (Layer 9 seam — v1 only RECORDS failures;
# restart/evolution policy lands with Layer 9).
# Install path on device: $PREFIX/var/lib/111/share/termux/cron-healthcheck.sh

set -u

PREFIX=${PREFIX:-/data/data/com.termux/files/usr}
STATE="$PREFIX/var/lib/111/state"
mkdir -p "$STATE"

if curl -fsS -m 10 http://127.0.0.1:11180/health >/dev/null 2>&1; then
	: # healthy: nothing to record
else
	printf '%s health check failed\n' "$(date -Iseconds 2>/dev/null || date)" >> "$STATE/health-failures.log"
fi
exit 0
