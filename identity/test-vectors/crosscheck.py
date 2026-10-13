#!/usr/bin/env python3
"""Independent cross-check of the frozen Layer 2 section 14 vectors (task T8).

The Rust conformance suite (crates/id111/tests/conformance.rs) proves the
vectors against the id111 crate. This script is the third leg: it re-derives
the vectors with NO id111 code and (almost) none of its dependency stack -
pure-Python big-integer curve arithmetic, hashlib, hmac, and the `cryptography`
package (OpenSSL) only for the RFC primitives (Ed25519 keygen/sign, X25519,
ChaCha20-Poly1305). Everything security-relevant that the crate decides - the
strict Ed25519 predicate, the X25519 low-order class, the announce/attestation/
supersedure rules, the section 6.4 recipe (HChaCha20 included), bech32, the
label - is re-implemented here from the spec text.

Usage:  python3 -I test-vectors/crosscheck.py          (exit 0 = every check holds)
Needs:  python3 >= 3.8, the `cryptography` package (tested with 3.4.8).
"""
import base64
import hashlib
import hmac
import json
import os
import struct
import sys

from cryptography.hazmat.primitives import serialization
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey
from cryptography.hazmat.primitives.asymmetric.x25519 import (
    X25519PrivateKey,
    X25519PublicKey,
)
from cryptography.hazmat.primitives.ciphers.aead import ChaCha20Poly1305

HERE = os.path.dirname(os.path.abspath(__file__))
CHECKS = 0
FAILS = []


def load(name):
    with open(os.path.join(HERE, name), encoding="utf-8") as f:
        return json.load(f)


def ok(cond, what):
    global CHECKS
    CHECKS += 1
    if not cond:
        FAILS.append(what)
        print("FAIL:", what)


def eq(got, want, what):
    ok(got == want, "%s\n    got  %r\n    want %r" % (what, got, want))


def h(s):
    return bytes.fromhex(s)


RAW = dict(encoding=serialization.Encoding.Raw, format=serialization.PublicFormat.Raw)


# ---------------------------------------------------------------- Ed25519 ---
P = 2**255 - 19
L = 2**252 + 27742317777372353535851937790883648493
D = (-121665 * pow(121666, P - 2, P)) % P
SQRT_M1 = pow(2, (P - 1) // 4, P)


def inv(x):
    return pow(x, P - 2, P)


def recover_x(y, sign):
    """Lenient on x=0 with sign=1 (mirrors dalek's decompress; such points are
    small-order and rejected by the strict predicate regardless)."""
    x2 = (y * y - 1) * inv(D * y * y + 1) % P
    if x2 == 0:
        return 0
    x = pow(x2, (P + 3) // 8, P)
    if (x * x - x2) % P != 0:
        x = x * SQRT_M1 % P
    if (x * x - x2) % P != 0:
        return None
    if (x & 1) != sign:
        x = P - x
    return x


BY = 4 * inv(5) % P
BX = recover_x(BY, 0)
B = (BX, BY, 1, BX * BY % P)
IDENT = (0, 1, 1, 0)


def padd(Pt, Q):
    a = (Pt[1] - Pt[0]) * (Q[1] - Q[0]) % P
    b = (Pt[1] + Pt[0]) * (Q[1] + Q[0]) % P
    c = 2 * Pt[3] * Q[3] * D % P
    d = 2 * Pt[2] * Q[2] % P
    e, f, g, hh = b - a, d - c, d + c, b + a
    return (e * f % P, g * hh % P, f * g % P, e * hh % P)


def pmul(s, Pt):
    Q = IDENT
    while s:
        if s & 1:
            Q = padd(Q, Pt)
        Pt = padd(Pt, Pt)
        s >>= 1
    return Q


def peq(Pt, Q):
    return (Pt[0] * Q[2] - Q[0] * Pt[2]) % P == 0 and (Pt[1] * Q[2] - Q[1] * Pt[2]) % P == 0


def compress(Pt):
    zi = inv(Pt[2])
    x, y = Pt[0] * zi % P, Pt[1] * zi % P
    return int.to_bytes(y | ((x & 1) << 255), 32, "little")


def decompress(enc):
    n = int.from_bytes(enc, "little")
    y, sign = n & ((1 << 255) - 1), n >> 255
    if y >= P:
        return None
    x = recover_x(y, sign)
    return None if x is None else (x, y, 1, x * y % P)


def small_order(Pt):
    return peq(pmul(8, Pt), IDENT)


def strict_verify(a_enc, msg, sig):
    """Section 3 strict predicate, from the spec text; returns the crate's
    VerifyError variant name or 'ok' (same check order as the crate)."""
    if (int.from_bytes(a_enc, "little") & ((1 << 255) - 1)) >= P:
        return "NonCanonicalPublicKey"
    if (int.from_bytes(sig[:32], "little") & ((1 << 255) - 1)) >= P:
        return "NonCanonicalR"
    s = int.from_bytes(sig[32:], "little")
    if s >= L:
        return "NonCanonicalS"
    A = decompress(a_enc)
    if A is None:
        return "MalformedPublicKey"
    R = decompress(sig[:32])
    if R is None or small_order(A) or small_order(R):
        return "InvalidSignature"
    k = int.from_bytes(hashlib.sha512(sig[:32] + a_enc + msg).digest(), "little") % L
    # cofactorless: [s]B == R + [k]A, compared on compressed encodings
    if compress(pmul(s, B)) != compress(padd(R, pmul(k, A))):
        return "InvalidSignature"
    return "ok"


def ed_pub(seed):
    return Ed25519PrivateKey.from_private_bytes(seed).public_key().public_bytes(**RAW)


def ed_sign(seed, msg):
    return Ed25519PrivateKey.from_private_bytes(seed).sign(msg)


# ----------------------------------------------------------------- X25519 ---
def x_pub(scalar):
    return X25519PrivateKey.from_private_bytes(scalar).public_key().public_bytes(**RAW)


def x_dh(scalar, u):
    return X25519PrivateKey.from_private_bytes(scalar).exchange(
        X25519PublicKey.from_public_bytes(u)
    )


def ladder(k, u):
    """RFC 7748 Montgomery ladder, UNCLAMPED scalar k (used for the cofactor)."""
    x1 = (int.from_bytes(u, "little") & ((1 << 255) - 1)) % P
    x2, z2, x3, z3, swap = 1, 0, x1, 1, 0
    for t in range(254, -1, -1):
        kt = (k >> t) & 1
        swap ^= kt
        if swap:
            x2, x3, z2, z3 = x3, x2, z3, z2
        swap = kt
        A = (x2 + z2) % P
        AA = A * A % P
        Bb = (x2 - z2) % P
        BB = Bb * Bb % P
        E = (AA - BB) % P
        C = (x3 + z3) % P
        Dd = (x3 - z3) % P
        DA = Dd * A % P
        CB = C * Bb % P
        x3 = (DA + CB) ** 2 % P
        z3 = x1 * (DA - CB) ** 2 % P
        x2 = AA * BB % P
        z2 = E * (AA + 121665 * E) % P
    if swap:
        x2, x3, z2, z3 = x3, x2, z3, z2
    return x2 * pow(z2, P - 2, P) % P


def low_order_x25519(u):
    return ladder(8, u) == 0


# ----------------------------------------------------------- BIP-39 / SLIP ---
def bip39_seed(words):
    return hashlib.pbkdf2_hmac("sha512", words.encode(), b"mnemonic", 2048)


def slip10(seed, indices):
    i = hmac.new(b"ed25519 seed", seed, hashlib.sha512).digest()
    key, chain = i[:32], i[32:]
    for idx in indices:
        data = b"\x00" + key + struct.pack(">I", idx | 0x80000000)
        i = hmac.new(chain, data, hashlib.sha512).digest()
        key, chain = i[:32], i[32:]
    return key, chain


def derive_keyset(words):
    seed = bip39_seed(words)
    sign = slip10(seed, [111, 0, 0])[0]
    ecdh = slip10(seed, [111, 0, 1])[0]
    return dict(seed=seed, sign_seed=sign, scalar=ecdh, node=ed_pub(sign), xpub=x_pub(ecdh))


# ----------------------------------------------------------------- bech32 ---
CHARSET = "qpzry9x8gf2tvdw0s3jn54khce6mua7l"
GEN = [0x3B6A57B2, 0x26508E6D, 0x1EA119FA, 0x3D4233DD, 0x2A1462B3]
BECH32_CONST, BECH32M_CONST = 1, 0x2BC830A3


def polymod(values):
    chk = 1
    for v in values:
        top = chk >> 25
        chk = (chk & 0x1FFFFFF) << 5 ^ v
        for i in range(5):
            chk ^= GEN[i] if ((top >> i) & 1) else 0
    return chk


def hrp_expand(hrp):
    return [ord(c) >> 5 for c in hrp] + [0] + [ord(c) & 31 for c in hrp]


def to5(data):
    acc, bits, out = 0, 0, []
    for b in data:
        acc, bits = (acc << 8) | b, bits + 8
        while bits >= 5:
            bits -= 5
            out.append((acc >> bits) & 31)
    if bits:
        out.append((acc << (5 - bits)) & 31)
    return out


def from5_lenient(vals):
    acc, bits, out = 0, 0, []
    for v in vals:
        acc, bits = (acc << 5) | v, bits + 5
        while bits >= 8:
            bits -= 8
            out.append((acc >> bits) & 0xFF)
        acc &= (1 << bits) - 1
    return bytes(out)


def bech32_encode(hrp, data, const=BECH32_CONST):
    vals = to5(data)
    pm = polymod(hrp_expand(hrp) + vals + [0] * 6) ^ const
    chk = [(pm >> 5 * (5 - i)) & 31 for i in range(6)]
    return hrp + "1" + "".join(CHARSET[v] for v in vals + chk)


def bech32_decode_node(s):
    """Token per the section 7 rules; mirrors the order of the crate's checks."""
    if s != s.lower() and s != s.upper():
        return "InvalidBech32"
    s = s.lower()
    pos = s.rfind("1")
    if pos < 1 or pos + 7 > len(s) or len(s) > 90:
        return "InvalidBech32"
    if any(c not in CHARSET for c in s[pos + 1 :]) or any(ord(c) < 33 or ord(c) > 126 for c in s):
        return "InvalidBech32"
    hrp, vals = s[:pos], [CHARSET.index(c) for c in s[pos + 1 :]]
    if polymod(hrp_expand(hrp) + vals) != BECH32_CONST:  # bech32m (0x2bc830a3) fails here
        return "InvalidBech32"
    if hrp != "node":
        return "WrongHrp"
    data = from5_lenient(vals[:-6])
    if len(data) != 32:
        return "InvalidLength:32:%d" % len(data)
    if bech32_encode("node", data) != s:
        return "NonCanonicalBech32"
    return "ok"


def label(key):
    b32 = base64.b32encode(hashlib.sha256(key).digest()).decode().lower().rstrip("=")[:10]
    return "%s-%s-%s" % (b32[:4], b32[4:8], b32[8:10])


def hex_token(s):
    if len(s) != 64:
        return "InvalidLength:64:%d" % len(s)
    return "ok" if all(c in "0123456789abcdef" for c in s) else "InvalidHex"


def record_token(rec):
    if len(rec) != 34:
        return "InvalidLength:34:%d" % len(rec)
    if rec[:2] not in (b"\xed\x01", b"\xec\x01"):
        return "UnknownPrefix:%s" % rec[:2].hex()
    return "ok"


# --------------------------------------------------------------- XChaCha20 ---
M32 = 0xFFFFFFFF


def rotl(v, n):
    return ((v << n) & M32) | (v >> (32 - n))


def hchacha20(key, nonce16):
    s = list(struct.unpack("<4I", b"expand 32-byte k")) + list(struct.unpack("<8I", key)) + list(
        struct.unpack("<4I", nonce16)
    )

    def qr(a, b, c, d):
        s[a] = (s[a] + s[b]) & M32; s[d] = rotl(s[d] ^ s[a], 16)
        s[c] = (s[c] + s[d]) & M32; s[b] = rotl(s[b] ^ s[c], 12)
        s[a] = (s[a] + s[b]) & M32; s[d] = rotl(s[d] ^ s[a], 8)
        s[c] = (s[c] + s[d]) & M32; s[b] = rotl(s[b] ^ s[c], 7)

    for _ in range(10):
        qr(0, 4, 8, 12); qr(1, 5, 9, 13); qr(2, 6, 10, 14); qr(3, 7, 11, 15)
        qr(0, 5, 10, 15); qr(1, 6, 11, 12); qr(2, 7, 8, 13); qr(3, 4, 9, 14)
    return struct.pack("<8I", *(s[0:4] + s[12:16]))


def xchacha_encrypt(key, nonce24, pt, aad):
    return ChaCha20Poly1305(hchacha20(key, nonce24[:16])).encrypt(b"\0" * 4 + nonce24[16:], pt, aad)


def xchacha_decrypt(key, nonce24, ct, aad):
    return ChaCha20Poly1305(hchacha20(key, nonce24[:16])).decrypt(b"\0" * 4 + nonce24[16:], ct, aad)


def seal_kdf(shared, epk, rpk):
    key = hashlib.blake2b(b"111/seal/v1" + shared + epk + rpk, digest_size=32).digest()
    nonce = hashlib.blake2b(b"111/seal-n/v1" + epk + rpk, digest_size=24).digest()
    return key, nonce


def seal_open_token(rsk, packet):
    """Section 6.4 open() from the spec text; returns the crate's token."""
    if len(packet) < 11 + 32 + 16:
        return "err:truncated"
    if packet[1] != 1:
        return "err:mode:%d" % packet[1]
    hl = 11 + packet[10]
    if len(packet) < hl + 32 + 16:
        return "err:truncated"
    epk = packet[hl : hl + 32]
    if low_order_x25519(epk):
        return "err:ecdh:LowOrderPublicKey"
    shared = x_dh(rsk, epk)
    if shared == b"\0" * 32:
        return "err:ecdh:NonContributory"
    key, nonce = seal_kdf(shared, epk, x_pub(rsk))
    try:
        inner = xchacha_decrypt(key, nonce, packet[hl + 32 :], packet[:hl])
    except Exception:
        return "err:auth"
    return "err:short-plaintext" if len(inner) < 32 else "ok"


# ============================================================ the checks ====
def check_ed25519():
    d = load("ed25519-rfc8032.json")
    for v in d["vectors"]:
        sk, pk, msg, sig = h(v["secret_key"]), h(v["public_key"]), h(v["message"]), h(v["signature"])
        eq(ed_pub(sk), pk, v["name"] + " pubkey")
        eq(ed_sign(sk, msg), sig, v["name"] + " signature")
        eq(strict_verify(pk, msg, sig), "ok", v["name"] + " strict verify")
        eq(strict_verify(pk, msg + b"\0", sig), "InvalidSignature", v["name"] + " wrong message")


def check_x25519():
    d = load("x25519-rfc7748.json")
    dh = d["diffie_hellman"]
    eq(x_pub(h(dh["alice_scalar"])), h(dh["alice_public"]), "alice pub")
    eq(x_pub(h(dh["bob_scalar"])), h(dh["bob_public"]), "bob pub")
    eq(x_dh(h(dh["alice_scalar"]), h(dh["bob_public"])), h(dh["shared"]), "alice dh")
    eq(x_dh(h(dh["bob_scalar"]), h(dh["alice_public"])), h(dh["shared"]), "bob dh")
    for v in d["scalarmult"]:
        eq(x_dh(h(v["scalar"]), h(v["u"])), h(v["output"]), "RFC 7748 5.2 vector")
    it = d["iterated"]
    k = u = h(it["start"])
    for i in range(1, 1001):
        k, u = x_dh(k, u), k
        if i == 1:
            eq(k.hex(), it["after_1"], "iterated x1")
    eq(k.hex(), it["after_1000"], "iterated x1000")


def check_slip10():
    d = load("slip0010-published.json")
    for v in d["vectors"]:
        for c in v["chains"]:
            key, chain = slip10(h(v["seed"]), c["indices"])
            eq((key.hex(), chain.hex()), (c["key"], c["chain"]), "%s %s" % (v["name"], c["path"]))
    d = load("slip0010-project-paths.json")
    for v in d["vectors"]:
        k = derive_keyset(v["mnemonic"])
        eq(k["seed"].hex(), v["bip39_seed"], v["name"] + " BIP-39 seed")
        eq(k["sign_seed"].hex(), v["signing_seed"], v["name"] + " m/111'/0'/0'")
        eq(k["scalar"].hex(), v["ecdh_scalar"], v["name"] + " m/111'/0'/1'")
        eq(len(h(v["entropy"])) * 8 * 33 // 32 // 11, len(v["mnemonic"].split()), v["name"] + " word count vs entropy")


def check_keyset():
    d = load("keyset-project.json")
    for v in d["vectors"]:
        k = derive_keyset(v["mnemonic"])
        eq(k["node"].hex(), v["node_id"], v["name"] + " NodeId")
        eq(k["xpub"].hex(), v["x25519_public"], v["name"] + " X25519 pub")
        eq((k["node"] + k["xpub"]).hex(), v["public_keyset"], v["name"] + " public keyset")
        eq((k["sign_seed"] + k["scalar"]).hex(), v["private_keyset"], v["name"] + " private keyset")


def check_encoding():
    d = load("encoding.json")
    for v in d["kats"]:
        key = h(v["key"])
        eq(v["hex"], key.hex(), v["name"] + " hex")
        eq(bech32_encode("node", key), v["bech32"], v["name"] + " bech32")
        eq(bech32_encode("node", key).upper(), v["bech32_qr"], v["name"] + " QR")
        eq(label(key), v["label"], v["name"] + " label")
        eq(bech32_decode_node(v["bech32"]), "ok", v["name"] + " decodes")
        eq(bech32_decode_node(v["bech32_qr"]), "ok", v["name"] + " QR decodes")
        eq((b"\xed\x01" + key).hex(), v["record_ed25519"], v["name"] + " record")
        eq((b"\xec\x01" + key).hex(), v["record_x25519"], v["name"] + " x record")
    neg = d["negatives"]
    for v in neg["record"]:
        eq(record_token(h(v["record"])), v["expect"], "record: " + v["name"])
    for v in neg["hex"]:
        eq(hex_token(v["input"]), v["expect"], "hex: " + v["name"])
    for v in neg["bech32"]:
        got = bech32_decode_node(v["input"])
        if len(v["input"]) > 90:
            # BIP-173 caps strings at 90 chars (reference decoder: InvalidBech32);
            # the Rust bech32 0.12 crate has no 90-char cap and rejects the same
            # string through the section 7 data-part-length rule instead. Both reject.
            ok(got == "InvalidBech32" and v["expect"].startswith("InvalidLength:32:"),
               "over-length bech32 rejected by both rules: " + v["name"])
        else:
            eq(got, v["expect"], "bech32: " + v["name"])
    # the bech32m negatives really carry the bech32m constant (not mislabelled)
    for v in neg["bech32"]:
        if "bech32m" in v["name"]:
            s = v["input"].lower()
            vals = [CHARSET.index(c) for c in s[s.rfind("1") + 1 :]]
            eq(polymod(hrp_expand("node") + vals), BECH32M_CONST, "bech32m constant: " + v["name"])


def check_edges():
    d = load("edge-vectors.json")
    eq(h(d["constants"]["group_order_l_le"]), L.to_bytes(32, "little"), "l constant")
    eq(h(d["constants"]["field_prime_p_le"]), P.to_bytes(32, "little"), "p constant")
    # torsion list: 8 distinct, each of order dividing 8, together the whole subgroup
    pts = []
    for t in d["torsion_subgroup"]:
        pt = decompress(h(t["encoding"]))
        ok(pt is not None and small_order(pt), "torsion decompresses+small-order: " + t["name"])
        eq(str(next(o for o in (1, 2, 4, 8) if peq(pmul(o, pt), IDENT))), t["order"], "order of " + t["name"])
        pts.append(pt)
    ok(len({compress(x) for x in pts}) == 7 or len({compress(x) for x in pts}) == 8, "torsion encodings (x=0 sign aliases)")
    # the dual-encoding constants: y and y+p are the same point
    can, nc = h(d["constants"]["dual_encoding_canonical_y"]), h(d["constants"]["dual_encoding_noncanonical_y_plus_p"])
    eq(int.from_bytes(nc, "little"), int.from_bytes(can, "little") + P, "y + p alias")
    ok(decompress(can) is not None, "dual-encoding point is on the curve")
    # every strict-predicate vector against the independent predicate
    for v in d["ed25519_strict"]:
        got = strict_verify(h(v["public_key"]), h(v["message"]), h(v["signature"]))
        eq("ok" if got == "ok" else "err:" + got, v["expect"], "strict: " + v["name"])
    for v in d["x25519_low_order"]:
        ok(low_order_x25519(h(v["u"])), "low-order: " + v["name"])
    for v in d["x25519_accepted_controls"]:
        ok(not low_order_x25519(h(v["u"])), "not low-order: " + v["name"])


def weak_token(k):
    def pattern(half):
        asc = all(half[i] == (half[0] + i) & 0xFF for i in range(32))
        dsc = all(half[i] == (half[0] - i) & 0xFF for i in range(32))
        return asc or dsc

    for half in (k[:32], k[32:]):
        if half == b"\0" * 32:
            return "AllZero"
        if len(set(half)) == 1:
            return "UniformByte"
        if pattern(half):
            return "CounterPattern"
    return "ok"


def check_weak():
    d = load("weak-key.json")
    for v in d["reject"]:
        eq(weak_token(h(v["private_keyset"])), v["expect"], "weak: " + v["name"])
    for v in d["accept"]:
        eq(weak_token(h(v["private_keyset"])), "ok", "weak accept: " + v["name"])


def ann_parse_token(wire, own):
    if not 137 <= len(wire) <= 169:
        return "err:length:%d" % len(wire)
    if wire[0] != 1:
        return "err:version:%d" % wire[0]
    if low_order_x25519(wire[33:65]):
        return "err:loworder"
    res = strict_verify(wire[1:33], b"111/announce/v1" + wire[:-64], wire[-64:])
    if res != "ok":
        return "err:sig:" + res
    if wire[1:33] == own:
        return "err:collision"
    return None  # passes parse+verify+collision; stateful rules next


def check_announce():
    d = load("announce.json")
    a, b = d["signers"]["a"], d["signers"]["b"]
    for who in (a, b):
        k = derive_keyset(who["mnemonic"])
        eq(k["node"].hex(), who["node_id"], "signer NodeId")
        eq(k["xpub"].hex(), who["enc_pubkey"], "signer enc pubkey")
    ka = derive_keyset(a["mnemonic"])
    kat = d["kat"]
    wire = h(kat["wire"])
    head = wire[:-64]
    eq(h(kat["signed_input"]), b"111/announce/v1" + head, "KAT signed input")
    eq(ed_sign(ka["sign_seed"], b"111/announce/v1" + head), wire[-64:], "KAT signature (independent signer)")
    eq(wire[0], 1, "version")
    eq(wire[1:33], ka["node"], "NodeId field")
    eq(wire[33:65], ka["xpub"], "enc field")
    eq(struct.unpack("<Q", wire[65:73])[0], int(kat["counter"]), "counter field")
    for v in d["positives"]:
        w = h(v["wire"])
        eq(ann_parse_token(w, h(b["node_id"])), None, "positive verifies: " + v["name"])
        eq(w[73:-64], h(v["appdata"]), "appdata: " + v["name"])
        eq(struct.unpack("<Q", w[65:73])[0], int(v["counter"]), "counter: " + v["name"])
    own_b = h(b["node_id"])
    for v in d["malformed"]:
        t = ann_parse_token(h(v["wire"]), own_b)
        eq(t, v["expect"], "malformed: " + v["name"])
    # stateful scenarios against an independent model of the section 6.2/6.3 rules
    for sc in d["scenarios"]:
        own, pins = h(sc["own_node_id"]), {}
        for i, step in enumerate(sc["steps"]):
            w = h(step["wire"])
            t = ann_parse_token(w, own)
            if t is None:
                node, enc, ctr = w[1:33], w[33:65], struct.unpack("<Q", w[65:73])[0]
                if node in pins:
                    penc, pctr = pins[node]
                    if penc != enc:
                        t = "err:conflict"
                    elif ctr <= pctr:
                        t = "err:stale:%d:%d" % (pctr, ctr)
                    else:
                        pins[node] = (enc, ctr)
                        t = "ok:updated:%d" % pctr
                else:
                    pins[node] = (enc, ctr)
                    t = "ok:pinned"
            eq(t, step["expect"], "scenario %s step %d" % (sc["name"], i))


def check_sealed():
    d = load("sealed-packet.json")
    r, k = d["recipient"], d["kat"]
    eq(x_pub(h(r["x25519_scalar"])), h(r["x25519_public"]), "recipient pub")
    esk, rpk = h(k["esk"]), h(k["rpk"])
    epk = x_pub(esk)
    shared = x_dh(esk, rpk)
    eq(epk, h(k["epk"]), "epk")
    eq(shared, h(k["shared"]), "shared")
    key, nonce = seal_kdf(shared, epk, rpk)
    eq(key.hex(), k["key"], "KDF key")
    eq(nonce.hex(), k["nonce"], "KDF nonce")
    hd = k["header"]
    hb = bytes([hd["version"], hd["crypto_mode"]]) + struct.pack("<Q", int(hd["counter"])) + bytes([len(h(hd["extra"]))]) + h(hd["extra"])
    eq(hb, h(k["header_bytes"]), "header bytes")
    inner = h(k["sender_node_id"]) + h(k["plaintext"])
    eq(inner, h(k["inner_plaintext"]), "inner plaintext")
    ct = xchacha_encrypt(key, nonce, inner, hb)
    eq(ct, h(k["ciphertext_and_tag"]), "ciphertext||tag")
    eq(hb + epk + ct, h(k["packet"]), "packet")
    rsk = h(r["x25519_scalar"])
    eq(seal_open_token(rsk, h(k["packet"])), "ok", "KAT opens")
    # wrong recipient (independent derivation of the other keyset)
    other = derive_keyset(d["wrong_recipient"]["mnemonic"])
    eq(seal_open_token(other["scalar"], h(k["packet"])), d["wrong_recipient"]["expect"], "wrong recipient")
    for v in d["negatives"]:
        eq(seal_open_token(rsk, h(v["packet"])), v["expect"], "sealed negative: " + v["name"])


def check_backup():
    d = load("backup-format.json")
    for v in d["kats"]:
        kk = derive_keyset(v["mnemonic"])
        ent = h(v["entropy"])
        desc = b"\x01\x01\x01" + (b"\x01" if len(ent) == 16 else b"\x02")
        eq(desc.hex(), v["descriptor"], v["name"] + " descriptor")
        eq((desc + ent).hex(), v["qr_payload"], v["name"] + " QR payload")
        eq(len(h(v["qr_payload"])), 20 if len(ent) == 16 else 36, v["name"] + " QR length")
        eq(v["node_id_hex"], kk["node"].hex(), v["name"] + " NodeId")
        eq(v["node_id_bech32"], bech32_encode("node", kk["node"]), v["name"] + " bech32")
        eq(v["label"], label(kk["node"]), v["name"] + " label")
        eq(v["nodeid_file"], (b"\xed\x01" + kk["node"]).hex(), v["name"] + " nodeid file")
        eq(v["keyset_file"], (kk["sign_seed"] + kk["scalar"]).hex(), v["name"] + " keyset file")
        eq(v["mnemonic_file_body"], v["mnemonic"] + "\n", v["name"] + " mnemonic file")
        eq(v["descriptor_file"], desc.hex(), v["name"] + " descriptor file")
    # NFKC hygiene vectors: the canonical phrase equals the NFKC-folded input
    import unicodedata
    for v in d["input_hygiene"]:
        folded = " ".join(unicodedata.normalize("NFKC", v["words"]).lower().split())
        eq(folded, v["canonical"], "hygiene: " + v["name"])
    # descriptor negatives: only 01 01 01 {01,02} of length 4 is valid
    for v in d["descriptor_negatives"]:
        b = h(v["bytes"])
        ok(not (len(b) == 4 and b[0] == 1 and b[1] == 1 and b[2] == 1 and b[3] in (1, 2)), "descriptor invalid: " + v["name"])
    for v in d["qr_negatives"]:
        b = h(v["payload"])
        valid = (len(b) == 20 and b[:4] == b"\x01\x01\x01\x01") or (len(b) == 36 and b[:4] == b"\x01\x01\x01\x02")
        ok(not valid, "QR payload invalid: " + v["name"])


def att_node_body(node, tkey, tid, expiry):
    return b"\x01" + node + tkey + bytes([tid]) + struct.pack("<Q", expiry)


def att_token(w):
    if len(w) != 202:
        return "err:length:%d" % len(w)
    if w[0] != 1:
        return "err:version:%d" % w[0]
    if w[33] != 1:
        return "err:tid:%d" % w[33]
    node, tkey, expiry = w[1:33], w[34:66], struct.unpack("<Q", w[66:74])[0]
    res = strict_verify(node, b"111/attest-node/v1" + att_node_body(node, tkey, w[33], expiry), w[74:138])
    return "ok" if res == "ok" else "err:node-sig:" + res


def check_attestation():
    d = load("attestation.json")
    k = d["kat"]
    ka = derive_keyset(d["signers"]["a"]["mnemonic"])
    node, tkey, expiry = h(k["node_pubkey"]), h(k["transport_key"]), int(k["expiry_epoch"])
    eq(node, ka["node"], "attestation node key")
    body = att_node_body(node, tkey, 1, expiry)
    eq(h(k["signed_input_node"]), b"111/attest-node/v1" + body, "node signed input (transport_key BEFORE transport_id)")
    eq(h(k["signed_input_transport"]), b"111/attest-transport/v1" + node, "transport signed input")
    eq(ed_sign(ka["sign_seed"], b"111/attest-node/v1" + body), h(k["node_sig"]), "node_sig (independent signer)")
    wire = b"\x01" + node + b"\x01" + tkey + struct.pack("<Q", expiry) + h(k["node_sig"]) + h(k["transport_sig_opaque"])
    eq(wire, h(k["wire"]), "attestation wire")
    eq(len(wire), 202, "202B")
    eq(struct.pack("<Q", expiry), h(k["expiry_epoch_le"]), "expiry LE")
    for v in d["negatives"]:
        eq(att_token(h(v["wire"])), v["expect"], "attestation negative: " + v["name"])
    # expiry: dead iff current >= expiry
    for c in d["expiry"]:
        cur = int(c["current_epoch"])
        want = ("err:expired:%d:%d" % (expiry, cur)) if cur >= expiry else "ok"
        eq(c["expect"], want, "expiry at %d" % cur)
    # supersedure model
    held = {}
    for s in d["supersedure"]:
        w = h(s["wire"])
        t = att_token(w)
        if t == "ok":
            key, ep = (w[1:33], w[33]), struct.unpack("<Q", w[66:74])[0]
            if key not in held:
                held[key], t = (ep, w), "ok:pinned"
            else:
                hep, hw = held[key]
                if ep > hep:
                    held[key], t = (ep, w), "ok:supersedes:%d" % hep
                elif ep < hep:
                    t = "err:superseded:%d:%d" % (hep, ep)
                elif w == hw:
                    t = "ok:duplicate"
                else:
                    t = "err:conflict:%d" % ep
        eq(t, s["expect"], "supersedure: " + s["name"])


def check_counters():
    d = load("restore-counters.json")
    eq(d["restore_jump"], 1024, "jump")
    for v in d["restore_reannounce"]:
        f = int(v["floor"])
        eq(int(v["announce_last"]), f + 1024, "announce_last floor %d" % f)
        eq(int(v["first_announce"]), f + 1024 + 1, "first announce floor %d" % f)
        eq(int(v["first_send_to_new_peer"]), 1025, "first send")
    ok(int(d["restore_edges"][0]["floor"]) + 1024 == 2**64 - 1, "edge: floor+1024 == u64::MAX")
    ok(int(d["restore_edges"][1]["floor"]) + 1024 > 2**64 - 1, "edge: floor+1024 overflows")
    ub = d["unclean_reboot"]
    eq(ub["last_committed_send"] + ub["batch_n"] + 1, ub["first_send_after_reboot"], "send jump J=N")
    eq(ub["last_committed_receive_high"] + ub["batch_n"], ub["receive_high_water_after_reboot"], "receive jump J=N")
    ok(ub["first_send_after_reboot"] > ub["sends_before_crash"], "J >= N clears every emitted counter")


def check_d9():
    d = load("d9-migration.json")
    k = derive_keyset(d["legacy"]["mnemonic"])
    eq(k["node"].hex(), d["expect"]["new_node_id"], "migrated NodeId (same words, new derivation)")
    eq(d["expect"]["new_mnemonic_file_body"], d["legacy"]["mnemonic"] + "\n", "mnemonic file body")
    eq(d["expect"]["announce_counter_init"], 0, "fresh counter")


def main():
    for fn in (check_ed25519, check_x25519, check_slip10, check_keyset, check_encoding, check_edges,
               check_weak, check_announce, check_sealed, check_backup, check_attestation,
               check_counters, check_d9):
        fn()
        print("ok  %s" % fn.__name__[len("check_"):])
    print("%d independent checks, %d failures" % (CHECKS, len(FAILS)))
    return 1 if FAILS else 0


if __name__ == "__main__":
    sys.exit(main())
