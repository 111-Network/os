#!/bin/sh
# 6-transport/verify/devices/fake-rnodeconf.sh — G5's fake `rnodeconf` (spec §12 identify step 1, §16 G5).
#
# devices.sh installs a copy as <root>/venvs/rns/bin/rnodeconf with @G5@ (the gate's state dir) and @GOLDENS@
# (1-bootstrap/crates/node111/tests/fixtures/rnodeconf: the tool's --info *text*, written from rnodeconf.py's output
# statements, never captured from a board) filled in. It knows nothing of RNodes and opens no port: no byte of it
# is modelled on Reticulum source (R06 G6). It logs its argv to @G5@/rnodeconf-calls.log, then prints the golden
# named by the first file in @G5@/rnodeconf/ whose name occurs in the port argument (the last argument), else
# no-rnode.txt (the NoMatch answer), and exits 0 like the real tool's graceful_exit().
G5='@G5@'
GOLD='@GOLDENS@'
printf '%s\n' "$*" >>"$G5/rnodeconf-calls.log"
port=""
for a in "$@"; do port=$a; done
answer=no-rnode.txt
for f in "$G5"/rnodeconf/*; do
  [ -f "$f" ] || continue
  key=$(basename "$f")
  case "$port" in *"$key"*) answer=$(cat "$f"); break ;; esac
done
cat "$GOLD/$answer"
exit 0
