#!/usr/bin/env python3
"""Generate supplement/cofactor-edges.json (harness-local, NOT a frozen vector file).

Run:  python3 -I gen_supplement.py > cofactor-edges.json     (python3 stdlib only)

Why this exists. The frozen edge-vectors.json builds its small-order and
non-canonical cases from RFC 8032 TEST 2's (R, S) with one field swapped, so the
signature equation is already false for most of them; a cofactored verifier
rejects them anyway. These supplement rows build inputs whose signature
equation HOLDS under the cofactored check, to show (a) that each section-3
wrapper leg is load-bearing against a validly-signed input, and (b) the former
residual (T9 F3): MIXED-ORDER R or A (prime-order part + a torsion component,
i.e. not "small-order") is accepted by any cofactored verifier. Spec v2.1
(2026-10-14) closed it by requiring A and R torsion-free ([l]P = O); the
"residual-mixed-order" kind name is kept for continuity, and the harness now
GATES those rows (the wrapper must reject them).

The oracle below is an independent pure-Python implementation of the section 3
predicate (v2.1) with the COFACTORLESS equation ([S]B == R + [h]A, point
equality): canonical A/R (y < p), S < l, A decodes, R decodes, small-order R/A
rejected, A and R torsion-free. Deterministic: no randomness, no clock.
"""
import hashlib
import json

p = 2**255 - 19
L = 2**252 + 27742317777372353535851937790883648493
d = -121665 * pow(121666, p - 2, p) % p
SQRT_M1 = pow(2, (p - 1) // 4, p)


def sha512(*parts):
    h = hashlib.sha512()
    for x in parts:
        h.update(x)
    return h.digest()


def recover_x(y, sign):
    if y >= p:
        return None
    x2 = (y * y - 1) * pow(d * y * y + 1, p - 2, p) % p
    if x2 == 0:
        return 0  # dalek (and Monocypher) decompress x = 0 with the sign bit set
    x = pow(x2, (p + 3) // 8, p)
    if (x * x - x2) % p != 0:
        x = x * SQRT_M1 % p
    if (x * x - x2) % p != 0:
        return None
    if (x & 1) != sign:
        x = p - x
    return x


def add(P, Q):
    A = (P[1] - P[0]) * (Q[1] - Q[0]) % p
    B = (P[1] + P[0]) * (Q[1] + Q[0]) % p
    C = 2 * P[3] * Q[3] * d % p
    D = 2 * P[2] * Q[2] % p
    E, F, G, H = B - A, D - C, D + C, B + A
    return (E * F % p, G * H % p, F * G % p, E * H % p)


IDENT = (0, 1, 1, 0)


def mul(s, P):
    Q = IDENT
    while s > 0:
        if s & 1:
            Q = add(Q, P)
        P = add(P, P)
        s >>= 1
    return Q


def eq(P, Q):
    return (P[0] * Q[2] - Q[0] * P[2]) % p == 0 and (P[1] * Q[2] - Q[1] * P[2]) % p == 0


def enc(P):
    zi = pow(P[2], p - 2, p)
    x, y = P[0] * zi % p, P[1] * zi % p
    return (y | ((x & 1) << 255)).to_bytes(32, "little")


def dec(b, tolerant=False):
    v = int.from_bytes(b, "little")
    sign, y = v >> 255, v & ((1 << 255) - 1)
    if tolerant:
        y %= p  # Monocypher accepts non-canonical y (reduced mod p)
    x = recover_x(y, sign)
    return None if x is None else (x, y % p, 1, x * (y % p) % p)


By = 4 * pow(5, p - 2, p) % p
B = (recover_x(By, 0), By, 1, recover_x(By, 0) * By % p)
assert eq(mul(L, B), IDENT)

# order-8 point (the (a) point of edge-vectors.json torsion_subgroup) and its multiples
T8 = dec(bytes.fromhex("c7176a703d4dd84fba3c0b760d10670f2a2053fa2c39ccc64ec7fd7792ac037a"))
T4 = mul(2, T8)
T2 = mul(4, T8)
assert eq(mul(8, T8), IDENT) and not eq(mul(4, T8), IDENT)
assert eq(mul(4, T4), IDENT) and not eq(mul(2, T4), IDENT)
assert eq(mul(2, T2), IDENT) and not eq(T2, IDENT)


def is_small_order(P):
    return eq(mul(8, P), IDENT)


def canonical(b):
    return (int.from_bytes(b, "little") & ((1 << 255) - 1)) < p


def is_torsion_free(P):
    return eq(mul(L, P), IDENT)


def strict_oracle(pk, sig, msg):
    """Section 3 predicate (v2.1), cofactorless. Returns 'ok' or the VerifyError variant."""
    R_enc, S_enc = sig[:32], sig[32:]
    if not canonical(pk):
        return "NonCanonicalPublicKey"
    if not canonical(R_enc):
        return "NonCanonicalR"
    if int.from_bytes(S_enc, "little") >= L:
        return "NonCanonicalS"
    A = dec(pk)
    if A is None:
        return "MalformedPublicKey"
    R = dec(R_enc)
    if R is None:
        return "InvalidSignature"
    if is_small_order(R) or is_small_order(A):
        return "InvalidSignature"
    if not is_torsion_free(A) or not is_torsion_free(R):  # v2.1
        return "InvalidSignature"
    h = int.from_bytes(sha512(R_enc, pk, msg), "little") % L
    S = int.from_bytes(S_enc, "little")
    lhs = mul(S, B)
    rhs = add(R, mul(h, A))
    return "ok" if eq(lhs, rhs) else "InvalidSignature"


def cofactored_oracle(pk, sig, msg):
    """What a cofactored, non-canonical-tolerant verifier (Monocypher core) decides."""
    R_enc, S_enc = sig[:32], sig[32:]
    S = int.from_bytes(S_enc, "little")
    A, R = dec(pk, True), dec(R_enc, True)
    if A is None or R is None or S >= L:
        return "reject"
    h = int.from_bytes(sha512(R_enc, pk, msg), "little") % L
    lhs = mul(8, add(mul(S, B), mul(h, (-A[0] % p, A[1], A[2], -A[3] % p))))
    rhs = mul(8, R)
    return "accept" if eq(lhs, rhs) else "reject"


# RFC 8032 TEST 1 key
SEED = bytes.fromhex("9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60")
hh = sha512(SEED)
a = (int.from_bytes(hh[:32], "little") & ((1 << 254) - 8)) | (1 << 254)
PREFIX = hh[32:]
A_pt = mul(a, B)
A_enc = enc(A_pt)
assert A_enc.hex() == "d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a"

MSG = b"111/monocypher-harness/supplement"


def sign_with(R_enc, r, pk_enc=A_enc, msg=MSG):
    """S = r + h*a (mod l) with h = H(R_enc || pk_enc || msg): valid iff R == [r]B (cofactorless)."""
    h = int.from_bytes(sha512(R_enc, pk_enc, msg), "little") % L
    S = (r + h * a) % L
    return R_enc + S.to_bytes(32, "little")


rows = []


def row(name, kind, pk, sig, msg, rationale):
    row_oracle = strict_oracle(pk, sig, msg)
    rows.append({
        "name": name,
        "kind": kind,
        "public_key": pk.hex(),
        "message": msg.hex(),
        "signature": sig.hex(),
        "oracle": "accept" if row_oracle == "ok" else "reject",
        "oracle_variant": row_oracle,
        "cofactored_verifier": cofactored_oracle(pk, sig, msg),
        "rationale": rationale,
    })


# 1. honest control
r0 = int.from_bytes(sha512(PREFIX, MSG), "little") % L
sig0 = sign_with(enc(mul(r0, B)), r0)
row("control: honest RFC 8032 TEST 1 key signs the supplement message", "control-valid", A_enc, sig0, MSG,
    "positive control: both a strict and a cofactored verifier accept")

# 2-4. wrapper legs biting on inputs whose cofactored equation holds
ident_enc = enc(IDENT)
row("small-order R (identity), r = 0, S = h*a: equation holds", "wrapper-bites", A_enc, sign_with(ident_enc, 0), MSG,
    "R = identity is small-order; [S]B = [h]A so even the cofactorless equation holds - ONLY the small-order R leg rejects it")
alias_ident = ((1 + p) | 0).to_bytes(32, "little")  # y = p + 1: non-canonical alias of the identity
assert alias_ident.hex().startswith("eeffff") and alias_ident[31] == 0x7f
row("non-canonical R alias (y = p + 1) of the identity, r = 0, S = h*a (h over the alias bytes)", "wrapper-bites", A_enc,
    sign_with(alias_ident, 0), MSG,
    "Monocypher accepts non-canonical R encodings; the canonical-R leg must reject this validly-signed input")
row("order-8 torsion R, S = h*a: holds only after cofactor clearing", "wrapper-bites", A_enc,
    sign_with(enc(T8), 0), MSG,
    "R is a small-order point of order 8: [8]-cleared equation holds (cofactored accepts); the small-order R leg must reject")
alias_nc_s = sig0[:32] + ((int.from_bytes(sig0[32:], "little") + L).to_bytes(32, "little"))
row("honest signature with S replaced by S + l", "wrapper-redundant", A_enc, alias_nc_s, MSG,
    "malleability: S + l verifies in a non-S-checking verifier; Monocypher itself already rejects S >= l, so this leg is redundant with the core (recorded, not required to bite)")

# 5-7. residual: mixed order (NOT small-order, NOT caught by any section-3 leg)
for tname, T in (("order-8", T8), ("order-2", T2)):
    Rm = add(mul(r0, B), T)
    Rm_enc = enc(Rm)
    assert not is_small_order(Rm)
    row("mixed-order R = [r]B + %s torsion, S = r + h*a" % tname, "residual-mixed-order", A_enc, sign_with(Rm_enc, r0), MSG,
        "R is not small-order and canonical; cofactored equation holds, cofactorless fails by exactly the torsion component. "
        "dalek verify_strict rejects; a cofactored verifier accepts")

# mixed-order A: A' = A + T8 (an attacker-chosen NodeId); honest key a signs with h over A'
Am = add(A_pt, T8)
Am_enc = enc(Am)
assert not is_small_order(Am)
for ctr in range(64):
    msg = MSG + b"/" + str(ctr).encode()
    r = int.from_bytes(sha512(PREFIX, msg), "little") % L
    Rm_enc = enc(mul(r, B))
    h = int.from_bytes(sha512(Rm_enc, Am_enc, msg), "little") % L
    if h % 8 != 0:  # cofactorless rejection needs [h]T8 != O
        break
else:
    raise SystemExit("no suitable counter")
row("mixed-order A' = A + order-8 torsion (attacker-chosen NodeId), honest signing key a", "residual-mixed-order", Am_enc,
    sign_with(Rm_enc, r, Am_enc, msg), msg,
    "A' is not small-order and canonical; the cofactored equation ignores the torsion part of A', the cofactorless one does not. "
    "A NodeId of this shape would verify on a cofactored (Monocypher) node and fail on a dalek verify_strict node")

doc = {
    "spec": "section 3 + section 14 (harness-local supplement; NOT a frozen vector file)",
    "generated": "2026-10-13",
    "description": (
        "Cofactor-class edge inputs for the Monocypher harness: validly-signed inputs that make each section-3 wrapper leg "
        "load-bearing, plus the mixed-order R/A cases that a cofactored verifier accepts and the strict predicate rejects "
        "(the T9 F3 residual, closed by spec v2.1's torsion-free A/R leg on 2026-10-14; the same rows are frozen in "
        "test-vectors/edge-vectors.json). 'oracle' is the independent pure-Python cofactorless section-3 v2.1 predicate in gen_supplement.py; "
        "'cofactored_verifier' is what a cofactored, non-canonical-tolerant verifier (the Monocypher core) decides. kind = "
        "control-valid (raw and wrapper accept) | wrapper-bites (harness GATES: raw accepts where the oracle rejects, wrapper rejects "
        "with exactly oracle_variant) | wrapper-redundant (harness GATES: wrapper rejects with oracle_variant; raw may also reject) | "
        "residual-mixed-order (harness prints a KNOWN-DELTA line and, since spec v2.1, GATES: the wrapper must reject with oracle_variant)."
    ),
    "provenance": "python3 -I gen_supplement.py; RFC 8032 TEST 1 key; deterministic; id111 verify_strict agreement recorded in test-vectors/monocypher-smoke.md",
    "vectors": rows,
}
print(json.dumps(doc, indent=2))
