#!/usr/bin/env python3
"""Independent re-derivation of the Layer 5 frozen vectors (spec §3 incl. the
sig-binds-fragment-position amendment, §4, §6.1, §6.2, §7.3) from the spec
text alone — no mesh111 code. Python 3.10 stdlib
only (hashlib BLAKE2b, RFC 7693 keyed mode). Ed25519 signatures are not
re-verified here (no stdlib Ed25519); the Rust loader checks them under
id111's strict predicate. Exit 0 = every derivable field agrees.

Usage: python3 -I crosscheck.py [dir]   (default: this file's directory)
"""
import hashlib
import json
import os
import sys

DIR = sys.argv[1] if len(sys.argv) > 1 else os.path.dirname(os.path.abspath(__file__))
FAILS = []


def check(name, got, want):
    if got != want:
        FAILS.append(f"{name}: got {got} want {want}")


def load(name):
    with open(os.path.join(DIR, name), encoding="utf-8") as f:
        return json.load(f)


def uvarint(v):
    out = bytearray()
    while True:
        low = v & 0x7F
        v >>= 7
        if v == 0:
            out.append(low)
            return bytes(out)
        out.append(low | 0x80)


def b2b(data, size, key=None):
    if key is None:
        return hashlib.blake2b(data, digest_size=size).digest()
    return hashlib.blake2b(data, digest_size=size, key=key).digest()


def seq(epoch, n):
    return uvarint(int(epoch)) + uvarint(int(n))


# ---------------------------------------------------------------- ids.json
ids = load("ids.json")
for v in ids["varints"]:
    check(f"varint {v['value']}", uvarint(int(v["value"])).hex(), v["hex"])
for s in ids["msg_seq"]:
    check(f"msg_seq {s['epoch']}/{s['n']}", seq(s["epoch"], s["n"]).hex(), s["hex"])

# ---------------------------------------------------------------- ack.json
def ack_block(b):
    body = uvarint(int(b["epoch"])) + uvarint(int(b["prefix"]))
    body += bytes([len(b["ranges"])])
    for e, st, ln in b["ranges"]:
        body += uvarint(int(e)) + uvarint(int(st)) + uvarint(int(ln))
    body += bytes([len(b["partials"])])
    for e, n, bm in b["partials"]:
        body += seq(e, n) + int(bm).to_bytes(2, "little")
    body += bytes([len(b["strong"])])
    for e, n, h in b["strong"]:
        body += seq(e, n) + bytes.fromhex(h)
    return uvarint(len(body)) + body


ack = load("ack.json")
for b in ack["blocks"]:
    check(f"ack block {b['name']}", ack_block(b).hex(), b["hex"])
for p in ack["payloads"]:
    acker = bytes.fromhex(ack["identities"][p["acker"]]["node_id"])
    sender = bytes.fromhex(ack["identities"][p["sender"]]["node_id"])
    blk = bytes.fromhex(p["block"])
    sig_in = acker + sender + uvarint(int(p["stream"])) + blk
    check(f"ack_sig input {p['name']}", sig_in.hex(), p["sig_input"])
    want_payload = blk + (bytes.fromhex(p["ack_sig"]) if p["ack_sig"] else b"")
    check(f"ack payload {p['name']}", want_payload.hex(), p["payload"])
for s in ack["strong_hashes"]:
    check(f"strong hash {s['payload']}", b2b(bytes.fromhex(s["payload"]), 16).hex(), s["hash"])

# ---------------------------------------------------------------- envelope.json
env = load("envelope.json")
idn = env["identities"]
pair = env["pair"]
ss = bytes.fromhex(pair["shared"])
a_enc = bytes.fromhex(idn["a"]["enc_pubkey"])
b_enc = bytes.fromhex(idn["b"]["enc_pubkey"])
check("pair key a->b", b2b(b"111/pair/v1" + ss + a_enc + b_enc, 32).hex(), pair["key_a_to_b"])
check("pair key b->a", b2b(b"111/pair/v1" + ss + b_enc + a_enc, 32).hex(), pair["key_b_to_a"])
keys = {("a", "b"): bytes.fromhex(pair["key_a_to_b"]), ("b", "a"): bytes.fromhex(pair["key_b_to_a"])}

PUBLIC = {0x11, 0x12, 0x13, 0x30}
for c in env["cases"]:
    name = c["name"]
    sender = bytes.fromhex(idn[c["sender"]]["node_id"])
    recipient = bytes.fromhex(idn[c["recipient"]]["node_id"])
    extra = bytes([c["priority"] << 6]) + recipient[:4]
    header = bytes([1, 1]) + int(c["counter"]).to_bytes(8, "little") + bytes([5]) + extra
    check(f"{name} header", header.hex(), c["header"])
    flags = 0
    body = b""
    if c["frag"] is not None:
        flags |= 0x01
        body += uvarint(c["frag"][0]) + uvarint(c["frag"][1])
    tlvs = []
    if c["seen_peer_seq"] is not None:
        v = seq(*c["seen_peer_seq"])
        tlvs.append(bytes([0x01, len(v)]) + v)
    for t, v in c["unknown_tlvs"]:
        v = bytes.fromhex(v)
        tlvs.append(bytes([t, len(v)]) + v)
    if tlvs:
        flags |= 0x02
        body += bytes([len(tlvs)]) + b"".join(tlvs)
    if c["ack"] is not None:
        flags |= 0x04
        body += bytes.fromhex(c["ack"])
    if c["signed"]:
        flags |= 0x08
    if c["receipt_strong"]:
        flags |= 0x10
    if c["card"] is not None:
        flags |= 0x20
        card = bytes.fromhex(c["card"])
        body += bytes([len(card)]) + card
    payload = bytes.fromhex(c["payload"])
    before = sender[:4] + bytes([c["kind"], flags]) + seq(c["epoch"], c["n"]) + body + payload
    check(f"{name} inner_before_auth", before.hex(), c["inner_before_auth"])
    key = keys[(c["sender"], c["recipient"])]
    epk = bytes.fromhex(c["epk"])
    mac = b2b(header + epk + before, 16, key=key)
    check(f"{name} pair_mac", mac.hex(), c["pair_mac"])
    inner = before + mac
    if c["signed"]:
        dest = bytes(32) if c["kind"] in PUBLIC else recipient
        fi, fc = (c["frag"][0], c["frag"][1]) if c["frag"] is not None else (0, 0)
        sig_in = (sender + dest + uvarint(0) + seq(c["epoch"], c["n"]) + bytes([c["kind"], flags])
                  + uvarint(fi) + uvarint(fc) + b2b(payload, 32))
        check(f"{name} sig_input", sig_in.hex(), c["sig_input"])
        inner += bytes.fromhex(c["sig"])
    check(f"{name} inner", inner.hex(), c["inner"])
    if c["budget"] is not None:
        bud = c["budget"]
        plen = len(header) + 32 + len(inner) + 16
        check(f"{name} packet_len", plen, bud["packet_len"])
        check(f"{name} fills bearer", plen + (1 if bud["shim"] else 0), bud["bearer"])

# ---------------------------------------------------------------- custody.json
cus = load("custody.json")
for r in cus["receipts"]:
    ref = b2b(b"111/custody/v1" + bytes.fromhex(r["item"]), 16)
    check(f"pkt_ref {r['name']}", ref.hex(), r["pkt_ref"])
    pl = bytes([r["status"]]) + uvarint(int(r["custody_seq"])) + ref
    if r["custody_sig"]:
        hub = bytes.fromhex(cus["identities"][r["hub"]]["node_id"])
        dest = bytes.fromhex(cus["identities"][r["dest"]]["node_id"])
        sig_in = hub + dest + bytes([r["status"]]) + uvarint(int(r["custody_seq"])) + ref
        check(f"custody sig_input {r['name']}", sig_in.hex(), r["sig_input"])
        pl += bytes.fromhex(r["custody_sig"])
    check(f"custody payload {r['name']}", pl.hex(), r["payload"])

# ---------------------------------------------------------------- enroll.json
enr = load("enroll.json")
for e in enr["tags"]:
    member = bytes.fromhex(enr["identities"][e["member"]]["node_id"])
    tag = b2b(b"111/enroll/v1" + member + e["code"].encode("ascii"), 16,
              key=bytes.fromhex(e["code_secret"]))
    check(f"enroll {e['name']}", tag.hex(), e["tag"])

# -------------------------------------------------------------- store-kdf.json
kdf = load("store-kdf.json")
for c in kdf["cases"]:
    secret = bytes.fromhex(c["store_secret"])
    node_id = bytes.fromhex(c["node_id"])
    key = b2b(b"111/store/v1" + node_id, 32, key=secret)
    check(f"store-kdf {c['name']}", key.hex(), c["store_key"])
    check(f"store-kdf sqlcipher literal {c['name']}", f"x'{key.hex()}'", c["sqlcipher_key"])

if FAILS:
    print("\n".join(FAILS))
    print(f"crosscheck: {len(FAILS)} mismatches")
    sys.exit(1)
print("crosscheck: all derivable fields agree")
