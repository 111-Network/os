#!/usr/bin/env python3
"""Stub Meshtastic stack for 4-interface/verify/panel-lint.sh (stdlib only; Layer 5 §14, G3).

meshmud (Go) is CI-built and there is no Go on the dev box, so the panel lint runs
111d against this stand-in. One file, three roles (select with --role; the wrappers
the lint installs under the scratch root pass it):

  --role meshmud   current/bin/meshmud            start-daemon -config <meshmud.toml>
                   Reads the config 111d rendered (uplink tcp address, downlink tcp
                   address) and does what meshmud does for the bits 111d can see:
                   LISTENs on the downlink (127.0.0.1:11230), dials the uplink (the
                   fake board below) and holds it, so 111d's ESTABLISHED-to-uplink
                   check passes. On the downlink it speaks the 0x94 0xC3 <len hi>
                   <len lo> framing, strictly (a byte that is not a frame header is
                   logged as a framing error — 111d's shim must only ever send whole
                   frames). JSON payloads stand in for protobufs (111d never decodes
                   them; tests/api_v1_meshtastic.rs has the same stub):
                     payload == b"want_config" -> debug noise, my_node_info, one node_info
                                                  frame per node, config_complete_id
                     any other payload P       -> one frame {"echo_hex": P.hex()}
  --role presence  current/share/meshtastic-presence/meshtastic_presence.py (behind
                   venvs/presence/bin/python): the real reader's CLI
                   (--endpoint host:port --out DIR), without the meshtastic library:
                   sends want_config once, writes DIR/meshtastic.json atomically from
                   the node_info frames once config_complete_id arrives. Without it the `meshtastic` service never
                   leaves "waiting for the board's list of nodes" and has no Panel tab.
  --role board     the fake board's TCP API (--listen host:port): accepts and holds.

Log lines go to stdout (111d appends them to meshtastic/meshmud.log); the lint greps:
  stub-meshmud: framing-error ...   (must be absent)
  stub-meshmud: frame-in ...        (what the shim sent)
"""
import argparse
import json
import os
import re
import socket
import sys
import threading
import time

START1, START2 = 0x94, 0xC3
WANT_CONFIG = b"want_config"
MY_NUM = 0x0A0A0A0A  # matches the binding the lint seeds (node_id !0a0a0a0a)

NOISE = b"DEBUG | 12:00:00 3 [Router] boot\r\n"


def my_node_info():
    return {
        "my_node_info": {
            "num": MY_NUM,
            "id": "!0a0a0a0a",
            "long_name": "Lint board",
            "short_name": "LB",
            "hw_model": "HELTEC_V3",
            "firmware_version": "2.7.11.abcdef0",
            "pio_env": "heltec-v3",
        }
    }


def nodes():
    def node(num, long_name, short_name, snr, rssi, hops, lat, lon):
        return {
            "id": "!%08x" % num,
            "num": num,
            "long_name": long_name,
            "short_name": short_name,
            "snr": snr,
            "rssi": rssi,
            "hops_away": hops,
            "last_heard_hint": 1_760_000_000 + num % 1000,
            "position": {"lat": lat, "lon": lon, "alt": 35},
        }

    return [
        node(0x1B2C3D4E, "Hilltop relay", "HT", 5.5, -97, 1, 51.5074, -0.1278),
        node(0x2C3D4E5F, "Ridge router", "RG", 2.25, -108, 2, 51.51, -0.12),
        node(0x3D4E5F60, "Valley sensor", "VS", -1.0, -117, 3, 51.49, -0.14),
    ]


def frame(payload):
    n = len(payload)
    return bytes([START1, START2, n >> 8, n & 0xFF]) + payload


def log(msg):
    print("stub-meshmud: " + msg, flush=True)


def config_reply():
    """What a Meshtastic node sends for want_config: my_node_info, one NodeInfo per node (each frame well under
    the 512 B FromRadio limit the shim enforces), then config_complete_id. Debug text first, like the board."""
    out = NOISE + frame(json.dumps(my_node_info()).encode())
    for n in nodes():
        out += frame(json.dumps({"node_info": n}).encode())
    return out + frame(json.dumps({"config_complete_id": 1}).encode())


# ---------------------------------------------------------------- config (a mini TOML)


def parse_config(path):
    """The two tables 111d renders: [[uplink.tcp]] address and [[downlink.tcp]] address."""
    cur = None
    out = {"uplink.tcp": [], "downlink.tcp": [], "uplink.serial": []}
    with open(path, encoding="utf-8") as f:
        for line in f:
            line = line.strip()
            if not line or line.startswith("#"):
                continue
            m = re.match(r"^\[\[([a-z.]+)\]\]$", line)
            if m:
                cur = {}
                out.setdefault(m.group(1), []).append(cur)
                continue
            m = re.match(r'^([a-z_]+)\s*=\s*(?:"([^"]*)"|(true|false))\s*$', line)
            if m and cur is not None:
                cur[m.group(1)] = m.group(2) if m.group(2) is not None else m.group(3) == "true"
    return out


def split_addr(addr):
    host, port = addr.rsplit(":", 1)
    return host.strip("[]"), int(port)


# ---------------------------------------------------------------- downlink


class Deframer:
    """Strict: anything that is not a frame header is a framing error."""

    def __init__(self):
        self.buf = b""

    def push(self, data):
        self.buf += data
        frames, errors = [], 0
        while len(self.buf) >= 4:
            if self.buf[0] != START1 or self.buf[1] != START2:
                errors += 1
                self.buf = b""
                break
            n = (self.buf[2] << 8) | self.buf[3]
            if len(self.buf) < 4 + n:
                break
            frames.append(self.buf[4 : 4 + n])
            self.buf = self.buf[4 + n :]
        return frames, errors


def serve_conn(conn, ident):
    log("downlink conn %d accepted" % ident)
    d = Deframer()
    try:
        while True:
            data = conn.recv(4096)
            if not data:
                break
            frames, errors = d.push(data)
            if errors:
                log("framing-error conn %d (bytes were not a 0x94 0xC3 frame)" % ident)
            for p in frames:
                shown = p.decode("utf-8", "replace") if len(p) <= 32 and p.isascii() else p.hex()[:64]
                log("frame-in conn %d len=%d %s" % (ident, len(p), shown))
                if p == WANT_CONFIG:
                    conn.sendall(config_reply())
                else:
                    conn.sendall(frame(json.dumps({"echo_hex": p.hex()}).encode()))
    except OSError:
        pass
    finally:
        conn.close()
        log("downlink conn %d closed" % ident)


def listen(host, port):
    s = socket.socket(socket.AF_INET6 if ":" in host else socket.AF_INET, socket.SOCK_STREAM)
    s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    s.bind((host, port))
    s.listen(16)
    return s


def role_meshmud(argv):
    if len(argv) != 3 or argv[0] != "start-daemon" or argv[1] != "-config" or not os.path.isfile(argv[2]):
        log("bad arguments: %s" % " ".join(argv))
        return 2
    cfg = parse_config(argv[2])
    if not cfg["downlink.tcp"] or not cfg["uplink.tcp"]:
        log("config has no tcp uplink/downlink: %r" % cfg)
        return 2
    dhost, dport = split_addr(cfg["downlink.tcp"][0]["address"])
    if cfg["downlink.tcp"][0].get("advertise") is not False:
        log("config does not pin advertise = false")
        return 2
    srv = listen(dhost, dport)
    log("listening on %s:%d (downlink); uplink %s" % (dhost, dport, cfg["uplink.tcp"][0]["address"]))

    uhost, uport = split_addr(cfg["uplink.tcp"][0]["address"])

    def hold_uplink():
        while True:
            try:
                u = socket.create_connection((uhost, uport), timeout=2)
                u.settimeout(None)
                log("uplink connected to %s:%d" % (uhost, uport))
                while u.recv(4096):
                    pass
                log("uplink closed")
            except OSError:
                time.sleep(0.5)

    threading.Thread(target=hold_uplink, daemon=True).start()
    n = 0
    while True:
        conn, _ = srv.accept()
        n += 1
        threading.Thread(target=serve_conn, args=(conn, n), daemon=True).start()


# ---------------------------------------------------------------- presence reader


def role_presence(argv):
    ap = argparse.ArgumentParser(prog="meshtastic_presence.py")
    ap.add_argument("--endpoint", required=True)
    ap.add_argument("--out", required=True)
    ap.add_argument("--once", action="store_true")
    ap.add_argument("--timeout", type=float, default=30)
    args = ap.parse_args(argv)
    host, port = split_addr(args.endpoint)
    while True:
        try:
            s = socket.create_connection((host, port), timeout=args.timeout)
            s.settimeout(None)
            s.sendall(frame(WANT_CONFIG))
            d = Deframer()
            buf = b""
            heard = []
            while True:
                data = s.recv(4096)
                if not data:
                    break
                # the board's debug text is not a frame: cut to the first header like the real client
                buf += data
                i = buf.find(bytes([START1, START2]))
                if i < 0:
                    buf = b""
                    continue
                frames, _ = d.push(buf[i:])
                buf = b""
                for p in frames:
                    msg = json.loads(p)
                    if "node_info" in msg:
                        heard = [n for n in heard if n["num"] != msg["node_info"]["num"]] + [msg["node_info"]]
                    if "config_complete_id" in msg:
                        tmp = os.path.join(args.out, ".meshtastic.json.tmp")
                        with open(tmp, "w", encoding="utf-8") as f:
                            json.dump({"schema": 1, "nodes": heard}, f)
                        os.replace(tmp, os.path.join(args.out, "meshtastic.json"))
                        log("presence: wrote %d nodes" % len(heard))
        except OSError:
            pass
        if args.once:
            return 3
        time.sleep(1)


# ---------------------------------------------------------------- fake board


def role_board(argv):
    ap = argparse.ArgumentParser()
    ap.add_argument("--listen", required=True)
    args = ap.parse_args(argv)
    host, port = split_addr(args.listen)
    srv = listen(host, port)
    log("board listening on %s:%d" % (host, port))
    held = []
    while True:
        conn, _ = srv.accept()
        held.append(conn)  # accept and hold; the board's own protocol is not spoken


def main():
    argv = sys.argv[1:]
    if len(argv) >= 2 and argv[0] == "--role":
        role, rest = argv[1], argv[2:]
        fn = {"meshmud": role_meshmud, "presence": role_presence, "board": role_board}.get(role)
        if fn:
            return fn(rest)
    sys.stderr.write(__doc__ or "")
    return 64


if __name__ == "__main__":
    try:
        sys.exit(main())
    except KeyboardInterrupt:
        sys.exit(0)
