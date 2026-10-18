#!/bin/sh
# ==========================================================================
# 111 Network — POSIX bootstrap shim (install.sh)
# Spec: os/10-bootstrap/README.md v2.1, "Architecture" §1.
#
# One-command install:
#   curl -fsSL https://install.111.network/install.sh | sh
#   curl -fsSL https://install.111.network/install.sh | sh -s -- --phone
#
# What this script does, in order:
#   1. detect OS / machine arch / userspace width
#      (refuses non-Linux, armv6/7 kernels, and unknown arches)
#   2. download the musl-static `111` binary for <arch>-unknown-linux-musl
#      (GitHub Releases is the primary/canonical source; the
#       install.111.network mirror is the fallback)
#   3. verify the download's sha256 against the pin embedded in THIS file,
#      inside a mktemp -d (mode 0700, trap-cleaned)
#   4. `install -m 0755` the verified binary to a root-owned path and exec
#      THAT copy — never the user-writable download (closes the
#      verify→exec TOCTOU)
#
# Privilege order: already root → sudo → doas → print the exact `su -c`
# line and exit 1. All shim args (--phone etc.) are passed through to the
# installer. Zero telemetry: the only outbound connections are the two
# artifact URLs.
#
# --phone: provisioning runs as the INVOKING user (adb authorization is
# per-user; the installer refuses root). Privilege is used for the
# `install -m 0755` step only — skipped entirely when the destination is
# user-writable — and the installed copy is exec'd WITHOUT sudo/doas. Its
# operator prompts read /dev/tty (stdin is this script's pipe under
# `curl | sh`); --yes answers them in advance.
#
# Anti-truncation: every statement lives inside a function; the ONLY
# top-level statement is the final `main "$@"` line. A partially-downloaded
# script therefore dies with a shell syntax error (or, at best, defines a
# few functions) without ever executing anything.
#
# Release machinery: the @@...@@ tokens in load_pins() are sed-replaced with
# real pins when a release is cut (scripts/gen-shim.sh, task T15). Never
# hand-edit them. They live inside a function so the file keeps exactly one
# top-level statement (see anti-truncation note above).
#
# TEST HOOKS (test-only; never set in production):
#   SHIM111_BASE_URL        replaces BOTH download bases when set
#   SHIM111_DRY_RUN=1       print the privileged commands instead of
#                           executing them (detection + download + sha256
#                           verification still happen for real)
#   SHIM111_FAKE_UNAME      pretend `uname -m` returned this value
#   SHIM111_FAKE_BIT        pretend `getconf LONG_BIT` returned this value
#   SHIM111_FAKE_PRIV=none  pretend: not root, no sudo, no doas
#   SHIM111_DEST            install destination instead of
#                           /usr/local/bin/111-bootstrap
#
# NOTE: hook names must be valid POSIX shell identifiers — an earlier draft
# used `111_*` names, but names starting with a digit are not importable
# (`export 111_FOO=x` is a syntax error) and busybox sh silently strips
# invalid-name environment entries on exec, breaking hook propagation.
# ==========================================================================

set -u

# ---------------------------------------------------------------------------
# output helpers
# ---------------------------------------------------------------------------

info() { printf '%s\n' "111-shim: $*"; }
warn() { printf '%s\n' "111-shim: WARNING: $*" >&2; }
die()  { printf '%s\n' "111-shim: error: $*" >&2; exit 1; }

have() { command -v "$1" >/dev/null 2>&1; }

# ---------------------------------------------------------------------------
# test hooks (see NOTE above)
# ---------------------------------------------------------------------------

read_hooks() {
	HOOK_BASE_URL=${SHIM111_BASE_URL:-}
	HOOK_DRY_RUN=${SHIM111_DRY_RUN:-}
	HOOK_FAKE_UNAME=${SHIM111_FAKE_UNAME:-}
	HOOK_FAKE_BIT=${SHIM111_FAKE_BIT:-}
	HOOK_FAKE_PRIV=${SHIM111_FAKE_PRIV:-}
	HOOK_DEST=${SHIM111_DEST:-}
}

# ---------------------------------------------------------------------------
# Release pins — sed-replaced by scripts/gen-shim.sh at release time.
# ---------------------------------------------------------------------------

load_pins() {
	SHIM_VERSION="@@SHIM_VERSION@@"
	SHA256_X86_64="@@SHA256_X86_64@@"
	SHA256_AARCH64="@@SHA256_AARCH64@@"
	MIN_INSTALLER_VERSION="@@MIN_INSTALLER_VERSION@@"

	GH_BASE="https://github.com/111-Network/os/releases/download/v${SHIM_VERSION}"
	MIRROR_BASE="https://install.111.network/v${SHIM_VERSION}"
	DEST="/usr/local/bin/111-bootstrap"
	DOCS="https://111.network/docs/install"
}

apply_dest_hook() {
	[ -n "$HOOK_DEST" ] && DEST=$HOOK_DEST
	:
}

# ---------------------------------------------------------------------------
# detection
# ---------------------------------------------------------------------------

detect_os() {
	OS=$(uname -s) || die "cannot determine the operating system (uname -s failed). See $DOCS"
	[ "$OS" = "Linux" ] || die "unsupported operating system: $OS — the 111 node runs on Linux only. See $DOCS"
}

detect_arch() {
	MACH=$HOOK_FAKE_UNAME
	[ -n "$MACH" ] || MACH=$(uname -m) || die "cannot determine the machine architecture (uname -m failed). See $DOCS"
	case "$MACH" in
		x86_64 | amd64)
			ARCH=x86_64 ;;
		aarch64 | arm64)
			ARCH=aarch64 ;;
		armv5* | armv6* | armv7* | arm)
			die "32-bit ARM machine architecture '$MACH' is not supported (armv7l hosts are refused per spec). Supported architectures: x86_64, aarch64. See $DOCS" ;;
		i?86)
			die "32-bit x86 machine architecture '$MACH' is not supported. Supported architectures: x86_64, aarch64. See $DOCS" ;;
		*)
			die "unknown architecture '$MACH'. Supported architectures: x86_64, aarch64. See $DOCS" ;;
	esac

	BITS=$HOOK_FAKE_BIT
	[ -n "$BITS" ] || BITS=$(getconf LONG_BIT 2>/dev/null || printf 'unknown')
	case "$BITS" in
		64)
			: ;;
		32)
			warn "32-bit userspace on a 64-bit $ARCH kernel — proceeding with the $ARCH (kernel-arch) binary, but a native 64-bit OS is strongly recommended. See $DOCS" ;;
		*)
			warn "could not determine userspace width (getconf LONG_BIT said '$BITS'); proceeding with the $ARCH binary" ;;
	esac
}

# ---------------------------------------------------------------------------
# workspace: mktemp -d, mode 0700, trap-cleaned
# ---------------------------------------------------------------------------

make_workspace() {
	TMPD=$(mktemp -d "${TMPDIR:-/tmp}/111-shim.XXXXXX") || die "mktemp -d failed"
	chmod 0700 "$TMPD" || die "chmod 0700 $TMPD failed"
	TMPBIN="$TMPD/111"
	trap 'rm -rf "$TMPD"' EXIT
	trap 'exit 129' HUP
	trap 'exit 130' INT
	trap 'exit 143' TERM
}

clear_traps() {
	trap - EXIT HUP INT TERM
}

# ---------------------------------------------------------------------------
# download: curl (preferred) or wget; GitHub Releases primary, mirror fallback
# ---------------------------------------------------------------------------

fetch() { # URL DEST
	if have curl; then
		curl -fsSL "$1" -o "$2"
	elif have wget; then
		wget -qO- "$1" > "$2"
	else
		return 127
	fi
}

download_binary() {
	TRIPLE="${ARCH}-unknown-linux-musl"
	if [ -n "$HOOK_BASE_URL" ]; then
		# TEST-ONLY hook: one base replaces BOTH the primary and fallback.
		URL_PRIMARY="${HOOK_BASE_URL%/}/111-${TRIPLE}"
		URL_FALLBACK="$URL_PRIMARY"
	else
		URL_PRIMARY="${GH_BASE}/111-${TRIPLE}"
		URL_FALLBACK="${MIRROR_BASE}/111-${TRIPLE}"
	fi

	have curl || have wget || die "neither curl nor wget found; cannot download the installer. See $DOCS"

	info "downloading 111 v${SHIM_VERSION} for ${TRIPLE}"
	if fetch "$URL_PRIMARY" "$TMPBIN"; then
		info "fetched from primary: $URL_PRIMARY"
	elif fetch "$URL_FALLBACK" "$TMPBIN"; then
		info "fetched from fallback: $URL_FALLBACK"
	else
		die "download failed from both primary and fallback URLs. See $DOCS"
	fi
	[ -s "$TMPBIN" ] || die "download produced an empty file; refusing to continue"
}

# ---------------------------------------------------------------------------
# sha256 verification against the embedded pin (sha256sum, busybox fallback)
# ---------------------------------------------------------------------------

sha256_of() { # FILE — prints "hash  filename"
	if have sha256sum; then
		sha256sum "$1"
	elif have busybox; then
		busybox sha256sum "$1"
	else
		return 127
	fi
}

verify_binary() {
	have sha256sum || have busybox || die "no sha256 tool available (need sha256sum or busybox); refusing to install an unverified binary. See $DOCS"
	case "$ARCH" in
		x86_64)  WANT="$SHA256_X86_64" ;;
		aarch64) WANT="$SHA256_AARCH64" ;;
		*)       die "internal error: no pin for arch '$ARCH'" ;;
	esac
	GOT=$(sha256_of "$TMPBIN") || die "sha256 computation failed"
	GOT=${GOT%% *}
	[ "$GOT" = "$WANT" ] || die "sha256 mismatch: expected $WANT, got $GOT — the download is corrupt or tampered; it will NOT be executed. See $DOCS"
	info "sha256 verified: $GOT"
}

# ---------------------------------------------------------------------------
# privilege escalation: root → sudo → doas → print exact su -c line, exit 1
# ---------------------------------------------------------------------------

detect_priv() {
	if [ "$HOOK_FAKE_PRIV" = "none" ]; then
		PRIV=none
	elif [ "$(id -u)" = "0" ]; then
		PRIV=root
	elif have sudo; then
		PRIV=sudo
	elif have doas; then
		PRIV=doas
	else
		PRIV=none
	fi
}

sq() { # print $1 wrapped in single quotes, safe for a su -c '...' line
	printf "'%s'" "$(printf '%s' "$1" | sed "s/'/'\\\\''/g")"
}

args_quoted() { # print each arg, space-separated and single-quoted
	_first=1
	for _a in "$@"; do
		[ "$_first" = 1 ] || printf ' '
		_first=0
		sq "$_a"
	done
}

has_phone_arg() { # any arg == --phone
	for _a in "$@"; do
		[ "$_a" = "--phone" ] && return 0
	done
	return 1
}

dest_user_writable() { # DEST can be (re)placed without privilege
	_dir=$(dirname "$DEST")
	[ -d "$_dir" ] && [ -w "$_dir" ] && { [ ! -e "$DEST" ] || [ -w "$DEST" ]; }
}

install_and_exec() {
	INSTALL_CMD="install -m 0755 $TMPBIN $DEST"
	EXEC_CMD="$DEST install --min-installer-version=$MIN_INSTALLER_VERSION"

	if has_phone_arg "$@"; then
		install_and_exec_phone "$@"
		return
	fi

	if [ "$PRIV" = "none" ]; then
		# Keep TMPD (traps cleared): the printed command references the
		# verified binary inside it.
		clear_traps
		warn "not root, and neither sudo nor doas was found — cannot escalate privileges."
		info "verified binary kept at: $TMPBIN"
		info "run this exact command as root to complete the install:"
		if [ $# -gt 0 ]; then
			printf '  su -c %s\n' "$(sq "$INSTALL_CMD && $EXEC_CMD $(args_quoted "$@")")"
		else
			printf '  su -c %s\n' "$(sq "$INSTALL_CMD && $EXEC_CMD")"
		fi
		exit 1
	fi

	if [ "$HOOK_DRY_RUN" = "1" ]; then
		info "dry-run: privilege mechanism: $PRIV"
		info "dry-run: would run: $INSTALL_CMD"
		if [ $# -gt 0 ]; then
			info "dry-run: would run: $EXEC_CMD $(args_quoted "$@")"
		else
			info "dry-run: would run: $EXEC_CMD"
		fi
		return 0
	fi

	case "$PRIV" in
		root) install -m 0755 "$TMPBIN" "$DEST" || die "install to $DEST failed" ;;
		sudo) sudo install -m 0755 "$TMPBIN" "$DEST" || die "sudo install to $DEST failed" ;;
		doas) doas install -m 0755 "$TMPBIN" "$DEST" || die "doas install to $DEST failed" ;;
		*)    die "internal error: bad privilege mechanism '$PRIV'" ;;
	esac
	info "installed root-owned $DEST"

	# tmpdir no longer needed; clean it now (exec skips the EXIT trap).
	rm -rf "$TMPD"
	clear_traps

	case "$PRIV" in
		root) exec "$DEST" install "--min-installer-version=$MIN_INSTALLER_VERSION" "$@" ;;
		sudo) exec sudo "$DEST" install "--min-installer-version=$MIN_INSTALLER_VERSION" "$@" ;;
		doas) exec doas "$DEST" install "--min-installer-version=$MIN_INSTALLER_VERSION" "$@" ;;
		*)    die "internal error: bad privilege mechanism '$PRIV'" ;;
	esac
}

# --phone: privilege for the binary install only; the installer itself runs as
# the invoking user (it refuses root — adb authorization is per-user).
install_and_exec_phone() {
	if [ "$PRIV" = "root" ]; then
		IPRIV=root
	elif dest_user_writable; then
		IPRIV=user
	else
		IPRIV=$PRIV
	fi

	if [ "$IPRIV" = "root" ]; then
		# Root shell (e.g. `curl … | sudo sh`): the phone flow cannot run as
		# root. Install the binary, then tell the user what to run.
		if [ "$HOOK_DRY_RUN" = "1" ]; then
			info "dry-run: would run: $INSTALL_CMD"
		else
			install -m 0755 "$TMPBIN" "$DEST" || die "install to $DEST failed"
			info "installed root-owned $DEST"
		fi
		warn "--phone must run as your normal user, not root (adb authorization is per-user)."
		info "run this as your normal user (no sudo) to provision the phone:"
		printf '  %s %s\n' "$EXEC_CMD" "$(args_quoted "$@")"
		exit 1
	fi

	if [ "$IPRIV" = "none" ]; then
		clear_traps
		warn "$DEST is not writable, and neither sudo nor doas was found — cannot install the binary."
		info "verified binary kept at: $TMPBIN"
		info "1. as root:              su -c $(sq "$INSTALL_CMD")"
		info "2. then as YOU, no sudo: $EXEC_CMD $(args_quoted "$@")"
		exit 1
	fi

	if [ "$HOOK_DRY_RUN" = "1" ]; then
		info "dry-run: privilege mechanism: $IPRIV (binary install step only)"
		info "dry-run: would run: $INSTALL_CMD"
		info "dry-run: would run as $(id -un), no sudo: $EXEC_CMD $(args_quoted "$@")"
		return 0
	fi

	case "$IPRIV" in
		user) install -m 0755 "$TMPBIN" "$DEST" || die "install to $DEST failed" ;;
		sudo) sudo install -m 0755 "$TMPBIN" "$DEST" || die "sudo install to $DEST failed" ;;
		doas) doas install -m 0755 "$TMPBIN" "$DEST" || die "doas install to $DEST failed" ;;
		*)    die "internal error: bad privilege mechanism '$IPRIV'" ;;
	esac
	info "installed $DEST"

	rm -rf "$TMPD"
	clear_traps

	# No sudo/doas here: the phone flow runs as you.
	exec "$DEST" install "--min-installer-version=$MIN_INSTALLER_VERSION" "$@"
}

# ---------------------------------------------------------------------------
# main
# ---------------------------------------------------------------------------

main() {
	load_pins
	read_hooks
	apply_dest_hook
	info "111 Network bootstrap shim v${SHIM_VERSION}"
	detect_os
	detect_arch
	make_workspace
	download_binary
	verify_binary
	detect_priv
	install_and_exec "$@"
}

# The ONLY top-level statement (anti-truncation): a partially-downloaded
# script never reaches this line.
main "$@"
