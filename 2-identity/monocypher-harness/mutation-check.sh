#!/usr/bin/env bash
# mutation-check.sh -- prove the harness can FAIL: apply one-line mutants to a throw-away
# copy of harness.c and require a non-zero exit for each. Uses python3 -I (stdlib) only to
# do exact-once string replacement. Not part of the gate; evidence for the smoke log.
#
#   bash os/2-identity/monocypher-harness/mutation-check.sh
#
# "Equivalent" mutants are expected to SURVIVE and are listed with the reason.
set -uo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
vectors="$(cd "$here/../test-vectors" && pwd)"
cc="${CC:-cc}"
work="$(mktemp -d "${TMPDIR:-/tmp}/monocypher-mutants.XXXXXX")"
trap 'rm -rf "$work"' EXIT

killed=0; survived=0; bad=0
run_mutant() { # label expect(kill|survive) old new
  local label="$1" expect="$2" old="$3" new="$4" d="$work/$1"
  mkdir -p "$d"
  if ! OLD="$old" NEW="$new" python3 -I - "$here/harness.c" "$d/harness.c" <<'PY'
import os, sys
s = open(sys.argv[1]).read()
o, n = os.environ["OLD"], os.environ["NEW"]
if s.count(o) != 1:
    sys.exit("pattern occurs %d times" % s.count(o))
open(sys.argv[2], "w").write(s.replace(o, n))
PY
  then echo "  ?? $label: mutation not applied"; bad=$((bad+1)); return; fi
  if ! "$cc" -std=c99 -O2 -Wall -Wextra -Werror -I"$here/vendor" -I"$here/vendor/optional" -o "$d/h" \
       "$d/harness.c" "$here/vendor/monocypher.c" "$here/vendor/optional/monocypher-ed25519.c" 2>"$d/cc.err"; then
    echo "  ?? $label: mutant does not compile"; bad=$((bad+1)); return; fi
  "$d/h" "$vectors" >"$d/out.txt" 2>&1; local rc=$?
  local nfail; nfail="$(grep -c '^  FAIL ' "$d/out.txt")"
  if [ "$expect" = kill ]; then
    if [ "$rc" -ne 0 ]; then echo "  killed    $label (exit $rc, $nfail failing checks)"; killed=$((killed+1))
    else echo "  SURVIVED  $label  <-- harness did not notice"; bad=$((bad+1)); fi
  else
    if [ "$rc" -eq 0 ]; then echo "  survived  $label (equivalent, as documented)"; survived=$((survived+1))
    else echo "  ??        $label: expected to survive but was killed"; bad=$((bad+1)); fi
  fi
}

echo "mutants that MUST be killed:"
run_mutant wrap-no-canonical-A kill 'if (!point_canonical(pk)) return V_NONCANON_A;' ''
run_mutant wrap-no-canonical-R kill 'if (!point_canonical(sig)) return V_NONCANON_R;' ''
run_mutant wrap-no-S-lt-l      kill 'if (!le_lt(sig + 32, GROUP_L)) return V_NONCANON_S;' ''
run_mutant wrap-no-small-order kill 'if (point_small_order(sig) || point_small_order(pk)) return V_INVALID;' ''
run_mutant wrap-no-A-decode    kill 'if (!point_decodes(pk)) return V_MALFORMED_A;' ''
run_mutant wrap-no-torsion-A    kill 'if (!point_torsion_free(pk)) return V_INVALID;  /* v2.1: mixed-order A (ghost NodeId A + T) */' ''
run_mutant wrap-no-torsion-R    kill 'if (!point_torsion_free(sig)) return V_INVALID; /* v2.1: mixed-order R (malleability) */' ''
run_mutant lP-times-8-not-l    kill 'ge_scalarmult(Q, P, GROUP_L);' 'static const uint8_t eight[32] = { 8 }; ge_scalarmult(Q, P, eight);'
run_mutant lP-no-sqrtm1-branch kill 'if (!gf_eq(chk, num)) gf_mul(x, x, ED_SQRTM1);' ''
run_mutant x25519-no-blacklist kill 'if (x25519_low_order(their_pk)) return E_LOWORDER;' ''
run_mutant seal-nonce-outlen32 kill 'crypto_blake2b_init(&c, 24);' 'crypto_blake2b_init(&c, 32);'
run_mutant seal-ad-empty       kill 'crypto_aead_lock(pkt + hl + 32, mac, key, nonce, pkt, hl, inner, 32 + pn);' 'crypto_aead_lock(pkt + hl + 32, mac, key, nonce, pkt, 0, inner, 32 + pn);'
run_mutant seal-mode-after-len kill 'if (mode != 1) { snprintf(tok, 64, "err:mode:%u", mode); return 0; }' ''
run_mutant announce-wrong-tag  kill '#define ANNOUNCE_TAG "111/announce/v1"' '#define ANNOUNCE_TAG "111/announce/v2"'
run_mutant attest-wire-order   kill '	memcpy(out + 33, w + 34, 32); /* transport_key */
	out[65] = w[33];              /* transport_id */' '	out[33] = w[33];
	memcpy(out + 34, w + 34, 32);'
run_mutant slip0010-no-harden  kill 'uint32_t h = idx[i] | 0x80000000u;' 'uint32_t h = idx[i];'
run_mutant pbkdf2-2047-rounds  kill 'for (int i = 1; i < 2048; i++) {' 'for (int i = 1; i < 2047; i++) {'

echo "equivalent mutants (expected to survive):"
run_mutant ecdh-no-allzero-abort survive 'if (all_zero(shared, 32)) { /* RFC 7748 6.1: abort, never return the secret */' 'if (0) {'
echo "(the all-zero abort is unreachable by design: the low-order ingress check fires first --"
echo " test-vectors/README.md 'Honest limits'. It is defence in depth, kept in the wrapper.)"

echo "summary: $killed killed, $survived equivalent survivors, $bad problems"
[ "$bad" -eq 0 ]
