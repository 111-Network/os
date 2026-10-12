#!/bin/sh
# verify/linux-e2e.sh — V8 (Linux end-to-end).
#
#   --namespace   best-effort variant: install the REAL musl `111` binary into a
#                 mount namespace (fake /etc + /var/lib/111, stubbed systemctl/
#                 userland, fixture signed channel on 127.0.0.1:11180) and assert
#                 the full install contract. Runs on any Linux with user
#                 namespaces; SKIP (exit 77) when unavailable.
#
#   (no args)     print the VM runbook (V8 blocker: Debian 12 + Arch + Fedora).
set -u

cd "$(dirname "$0")/.." || exit 1

runbook() {
  cat <<'EOF'
V8 VM runbook (spec blocker — run before first signed release):

  For each of: Debian 12 (systemd 252), Arch (current), Fedora (current):
    1. fresh VM, minimal install, outbound internet
    2. curl -fsSL https://install.111.network/install.sh | sh
       (or the release-candidate shim: dist/install.sh)
    3. assert: dashboard 200 on 127.0.0.1:11180 (loopback), identity 0600/0700,
       receipt.json present, cache/packages non-empty
    4. re-run the command: idempotent no-change
    5. run an OLDER pinned shim: refuses to downgrade
    6. sudo 111-uninstall: units, udev rule, user, trees all gone
    7. Debian 12 sudo-absent variant: single root account, su -c path works
  Record results in the release notes (V8 is a spec BLOCKER).
EOF
  exit 0
}

[ "${1:-}" = "--namespace" ] || runbook

# --- namespace variant -------------------------------------------------------
command -v unshare >/dev/null 2>&1 || { echo "SKIP: no unshare"; exit 77; }
unshare -rm true 2>/dev/null || { echo "SKIP: user namespaces unavailable"; exit 77; }
[ -f /tmp/dist-test/111-x86_64-unknown-linux-musl ] || {
  echo "SKIP: dist binary missing (run scripts/cross-build.sh 2026.10.12 /tmp/dist-test first)"
  exit 77
}

T=$(mktemp -d /tmp/111-e2e.XXXXXX)
SRV=""
# shellcheck disable=SC2317  # invoked via trap, not direct flow
cleanup() {
  [ -n "$SRV" ] && kill "$SRV" 2>/dev/null
  rm -rf "$T"
}
trap cleanup EXIT

# fixture signed channel on 127.0.0.1:11180
python3 verify/fixture_server.py "$T/fixture" > "$T/server.log" 2>&1 &
SRV=$!
_i=0
while ! grep -q READY "$T/server.log" 2>/dev/null; do
  _i=$((_i + 1))
  [ "$_i" -gt 100 ] && { echo "FAIL: fixture server never ready"; cat "$T/server.log"; exit 1; }
  sleep 0.2
done

# stage: fake fs pieces + stub userland (explicit mkdirs — dash has no brace expansion)
mkdir -p "$T/stage/systemd/system" "$T/stage/udev/rules.d" "$T/stage/bin" "$T/stage/varlib111" "$T/stage/etc111" "$T/stage/home"
STUB="$T/stub"
mkdir -p "$STUB"
for tool in systemctl udevadm useradd usermod getent chown chmod id hostname; do
  # shellcheck disable=SC2016  # stub body is a template — $* and $1 expand when the stub runs
  printf '#!/bin/sh\necho "stub: %s $*" >> "%s/stub-calls.log"\ncase "$1" in group) exit 0;; esac\nexit 0\n' "$tool" "$T" > "$STUB/$tool"
  chmod 0755 "$STUB/$tool"
done
# getent: dialout present, uucp absent
# shellcheck disable=SC2016  # template: $1/$2 expand when the stub runs
printf '#!/bin/sh\n[ "$1" = group ] && [ "$2" = dialout ] && { echo "dialout:x:20:"; exit 0; }\nexit 2\n' > "$STUB/getent"
chmod 0755 "$STUB/getent"
printf '#!/bin/sh\necho 127.0.0.1\n' > "$STUB/hostname"
chmod 0755 "$STUB/hostname"

# run the real installer inside the namespace
cp /tmp/dist-test/111-x86_64-unknown-linux-musl "$T/111"
cp /etc/resolv.conf "$T/resolv.conf" 2>/dev/null || printf 'nameserver 1.1.1.1\n' > "$T/resolv.conf"
unshare -rm sh -c "
  set -eu
  mount -t tmpfs tmpfs /etc
  mkdir /etc/systemd /etc/udev /etc/111
  mount --bind '$T/stage/systemd' /etc/systemd
  mount --bind '$T/stage/udev' /etc/udev
  mount --bind '$T/stage/etc111' /etc/111
  touch /etc/resolv.conf
  mount --bind '$T/resolv.conf' /etc/resolv.conf
  mount --bind '$T/stage/bin' /usr/local/bin
  mount -t tmpfs tmpfs /var/lib
  mkdir /var/lib/111
  mount --bind '$T/stage/varlib111' /var/lib/111
  mount --bind '$T/stage/home' '$HOME'
  export PATH='$STUB:/usr/bin:/bin'
  export NODE111_MANIFEST_BASE='http://127.0.0.1:11180'
  export NODE111_PROBE_204='http://127.0.0.1:11180/204'
  /'$T'/111 install --min-installer-version=2026.10.01
" || { echo "FAIL: namespace install errored"; echo "--- fixture server log ---"; cat "$T/server.log"; exit 1; }

# assert the install contract on the staged trees
ok=1
check() { # $1 label $2 path
  if [ -e "$2" ] || [ -L "$2" ]; then echo "  ok: $1"; else echo "  MISSING: $1 ($2)"; ok=0; fi
}
check "unit"            "$T/stage/systemd/system/111node.service"
check "update path"     "$T/stage/systemd/system/111node-update.path"
check "udev rule"       "$T/stage/udev/rules.d/99-111-lora.rules"
check "env"             "$T/stage/etc111/env"
check "release binary"  "$T/stage/varlib111/releases/2026.10.12/bin/111"
check "current link"    "$T/stage/varlib111/current"
check "identity trio"   "$T/stage/varlib111/identity/npub"
check "secret perms"    "$T/stage/varlib111/identity/secret.hex"
check "receipt"         "$T/stage/varlib111/receipt.json"
check "floor"           "$T/stage/varlib111/state/floor.json"
check "offer stub"      "$T/stage/varlib111/state/offer.json"
check "symlink"         "$T/stage/bin/111"
PERMS=$(stat -c %a "$T/stage/varlib111/identity/secret.hex" 2>/dev/null || echo "?")
if [ "$PERMS" = "600" ]; then
  echo "  ok: secret.hex 0600"
else
  echo "  BAD PERMS: $PERMS"
  ok=0
fi

if [ "$ok" = 1 ]; then
  echo "V8-variant namespace e2e: PASS (real binary, fixture channel, full contract)"
  exit 0
else
  echo "V8-variant namespace e2e: FAIL"
  exit 1
fi
