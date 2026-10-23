#!/usr/bin/env bash
# 6-transport/verify/zero-weight.sh — Layer 6 gate G7, zero weight (spec §16 G7, §2.1 rule 2; D26 #1).
#
# With every optional component disabled: nothing is installed (no <root>/components/, no venvs/<component>), the
# processes under 111d and the TCP ports they listen on equal the Layer 5 baseline (verify/baseline-listeners.txt),
# and the idle RSS of 111d and the cached payload sizes are RECORDED (note:), not asserted.
#
#   NODE111_ROOT=/var/lib/111 bash 6-transport/verify/zero-weight.sh            the daemon is found by its cmdline
#   NODE111_ROOT=… NODE111_PID=<111d pid> bash 6-transport/verify/zero-weight.sh
#   REQUIRE_G7=1 …                                                                lock run: no daemon is a FAIL
#
# Reads /proc only (no ss, no ip): the listener set comes from /proc/<pid>/net/tcp{,6} (the daemon's own network
# namespace) joined with the socket inodes of 111d and its descendants, so it must run as the daemon's user or root.
# A descendant counts only when it is there in two samples a second apart (111d's own short probes come and go).
# First cut (S4); V4 runs it on the live node. Starts nothing, so nothing of ours stays running.
set -u
export LC_ALL=C
cd "$(dirname "$0")/../.." || exit 1
. 6-transport/verify/lib.sh

BASELINE=6-transport/verify/baseline-listeners.txt
DECLS=1-bootstrap/crates/node111/assets/components
ROOT=${NODE111_ROOT:-}

[ -n "$ROOT" ] || skip_or_fail G7 "NODE111_ROOT is not set (the daemon root to inspect)"
[ -d "$ROOT" ] || skip_or_fail G7 "NODE111_ROOT=$ROOT is not a directory"
[ -r "$BASELINE" ] || { fail "missing $BASELINE"; finish G7; }

cmdline() { tr '\0' ' ' < "/proc/$1/cmdline" 2>/dev/null; }

# ---- the daemon: NODE111_PID, else the one `111 daemon` (whose NODE111_ROOT matches when readable)
find_daemon() {
  local p argv0 argv1 found=""
  for p in /proc/[0-9]*; do
    p=${p#/proc/}
    { IFS= read -r -d '' argv0 && IFS= read -r -d '' argv1; } < "/proc/$p/cmdline" 2>/dev/null || continue
    [ "${argv0##*/}" = 111 ] && [ "$argv1" = daemon ] || continue
    if [ -r "/proc/$p/environ" ] && tr '\0' '\n' < "/proc/$p/environ" | grep -q '^NODE111_ROOT='; then
      tr '\0' '\n' < "/proc/$p/environ" | grep -qx "NODE111_ROOT=$ROOT" || continue
    fi
    found="$found $p"
  done
  set -- $found
  [ $# -eq 1 ] && printf '%s' "$1"
}
PID=${NODE111_PID:-$(find_daemon)}
[ -n "$PID" ] || skip_or_fail G7 "no running 111d for $ROOT (or more than one; set NODE111_PID)"
[ -r "/proc/$PID/cmdline" ] || skip_or_fail G7 "pid $PID is not running"
ls "/proc/$PID/fd" >/dev/null 2>&1 || skip_or_fail G7 "cannot read /proc/$PID/fd (run as the daemon's user or root)"
note "111d pid $PID: $(cmdline "$PID")"

# ---- component ids declared by this tree (assets/components/<id>.toml)
IDS=$(sed -n 's/^id[[:space:]]*=[[:space:]]*"\([a-z0-9_-]*\)".*/\1/p' "$DECLS"/*.toml 2>/dev/null | sort -u)
[ -n "$IDS" ] || fail "no component declarations found under $DECLS"

# ---- 1. nothing installed
if [ -e "$ROOT/components" ] || [ -L "$ROOT/components" ]; then
  fail "$ROOT/components exists (an installed component; remove its files)"
else
  ok "no $ROOT/components"
fi
for id in $IDS; do
  for v in "$ROOT/venvs/$id" "$ROOT/venvs/$id.d"; do
    if [ -e "$v" ] || [ -L "$v" ]; then fail "$v exists (component $id is installed)"; fi
  done
done
ok "no venvs/<component> for: $(echo $IDS)"

# ---- 2. processes under 111d equal the baseline
descendants() { # every pid whose ancestry reaches $1 (from /proc/*/stat ppid)
  awk -v root="$1" '
    { ppid[$1] = $2 }
    END {
      n = 0; queue[n++] = root
      for (i = 0; i < n; i++) for (p in ppid) if (ppid[p] == queue[i] && !(p in seen)) { seen[p] = 1; queue[n++] = p; print p }
    }' < <(for s in /proc/[0-9]*/stat; do
            line=$(cat "$s" 2>/dev/null) || continue
            pid=${line%% *}; rest=${line##*) }; set -- $rest; echo "$pid $2"
          done)
}
BASE_PROCS=$(awk '$1 == "proc" { print $2 "\t" $3 }' "$BASELINE")
classify() { # pid -> baseline name, or "?"
  local c name match
  c=$(cmdline "$1")
  for id in $IDS; do
    case "$c" in *"$ROOT/components/"*|*"/venvs/$id/"*|*"/venvs/$id.d/"*) echo "component:$id"; return;; esac
  done
  while IFS=$'\t' read -r name match; do
    [ -n "$name" ] || continue
    case "$c" in *"$match"*) echo "$name"; return;; esac
  done <<< "$BASE_PROCS"
  echo "?"
}
S1=$(descendants "$PID" | sort -n)
sleep 1
S2=$(descendants "$PID" | sort -n)
STABLE=$(comm -12 <(echo "$S1") <(echo "$S2") | sed '/^$/d')
RUNNING=""
for p in $STABLE; do
  what=$(classify "$p")
  case "$what" in
    "?") fail "process $p under 111d is not in the Layer 5 baseline: $(cmdline "$p")" ;;
    component:*) fail "process $p is ${what} (a component runs while G7 expects all disabled): $(cmdline "$p")" ;;
    *) ok "baseline process $what (pid $p)"; RUNNING="$RUNNING $what" ;;
  esac
done
[ -n "$STABLE" ] || note "111d has no child processes"

# ---- 3. TCP listeners of 111d and its descendants equal the baseline
socket_inodes() { # pid... -> "inode pid" lines
  local p f t
  for p in "$@"; do
    for f in /proc/"$p"/fd/*; do
      t=$(readlink "$f" 2>/dev/null) || continue
      case "$t" in socket:\[*\]) t=${t#socket:[}; echo "${t%]} $p";; esac
    done
  done
}
INODES=$(socket_inodes "$PID" $STABLE)
sockets() { # /proc/net table, state filter ("" = any) -> "inode port" lines (ports from hex, in bash: mawk has no strtonum)
  [ -r "$1" ] || return 0
  awk -v st="$2" 'NR > 1 && (st == "" || $4 == st) { n = split($2, a, ":"); print $10, a[n] }' "$1" |
    while read -r ino hex; do echo "$ino $((16#$hex))"; done
}
LISTEN=$( { sockets "/proc/$PID/net/tcp" 0A; sockets "/proc/$PID/net/tcp6" 0A; } | sort -u)
OWNED=$(join <(echo "$INODES" | sort -k1,1) <(echo "$LISTEN" | sort -k1,1) 2>/dev/null | awk '{ print $3, $2 }' | sort -un)
running() { case " $RUNNING " in *" $1 "*) return 0;; esac; return 1; }
fabric_listen() {
  local s="$ROOT/fabric/settings"
  [ -e "$s" ] || { note "no $s (a Cat 4 hub defaults to hub_role): 11183 allowed"; return 0; }
  grep -Eq '"(hub_role|listen_lan)"[[:space:]]*:[[:space:]]*true' "$s"
}
allowed() { # port -> 0 when a baseline line allows it now
  local port=$1 kind range when holder lo hi
  while read -r kind range when holder; do
    [ "$kind" = listen ] || continue
    lo=${range%-*}; hi=${range#*-}
    [ "$port" -ge "$lo" ] && [ "$port" -le "$hi" ] || continue
    case "$when" in
      always | optional) return 0 ;;
      child:*) running "${when#child:}" && return 0 ;;
      fabric_listen) fabric_listen && return 0 ;;
      panel) running meshmud && return 0 ;;
    esac
  done < <(grep -v '^[[:space:]]*#' "$BASELINE")
  return 1
}
SEEN=""
while read -r port owner; do
  [ -n "$port" ] || continue
  SEEN="$SEEN $port"
  if allowed "$port"; then ok "listener $port (pid $owner) is in the baseline"
  else fail "listener $port (pid $owner: $(cmdline "$owner")) is not in the Layer 5 baseline"; fi
done <<< "$OWNED"
for port in $(awk '$1 == "listen" && $3 == "always" { print $2 }' "$BASELINE"); do
  case " $SEEN " in *" $port "*) ;; *) fail "listener $port (always) is not listening";; esac
done

# ---- 4. recorded, not asserted: idle RSS, cache sizes, UDP sockets
rss() { awk '/^VmRSS:/ { print $2 }' "/proc/$1/status" 2>/dev/null; }
note "RSS 111d: $(rss "$PID") kB"
for p in $STABLE; do note "RSS $(classify "$p") (pid $p): $(rss "$p") kB"; done
for id in $IDS; do
  d="$ROOT/cache/packages/$id"
  if [ -d "$d" ]; then note "cache/packages/$id: $(du -sb "$d" | cut -f1) bytes cached, not installed"
  else note "cache/packages/$id: nothing cached"; fi
done
[ -d "$ROOT/cache/packages" ] && note "cache/packages total: $(du -sb "$ROOT/cache/packages" | cut -f1) bytes"
UDP=$( { sockets "/proc/$PID/net/udp" ""; sockets "/proc/$PID/net/udp6" ""; } | sort -u)
UDP_OWNED=$(join <(echo "$INODES" | sort -k1,1) <(echo "$UDP" | sort -k1,1) 2>/dev/null | awk '{ print $3 }' | sort -un | tr '\n' ' ')
note "UDP ports bound under 111d (recorded): ${UDP_OWNED:-none}"

finish G7
