#!/usr/bin/env python3
"""G5 stand-ins for meshmud and the Meshtastic presence reader (spec §12 identify step 2, §16 G5; stdlib only).

meshmud (Go) is CI-built and there is no Go on this box, so 6-transport/verify/devices.sh runs 111d's Meshtastic
identify step against these. The shapes are the ones 111d's own tests use (tests/devices_core.rs `stub_downlink`,
`FAKE_READER`; 4-interface/verify/stub-meshmud.py): the downlink speaks the 0x94 0xC3 <len hi> <len lo> framing with
JSON payloads standing in for protobufs (111d never decodes the identify answer itself: the reader does).

  --role meshmud   current/bin/meshmud start-daemon -config <meshmud.toml>
                   Reads the config 111d rendered (the serial uplink `device`, the TCP downlink `address`), never
                   opens the board port, LISTENs on the downlink and answers a `want_config` frame per the mode of
                   the board: the first file in <g5>/meshmud/ whose name occurs in the uplink path holds the mode
                     match      → one {"my_node_info": …} frame (num/id from the file's second line, else 0x0a0a0a0a)
                     nomatch    → the connection is closed (the reader's --once then exits 3: "not Meshtastic")
                     slow:<s>   → <s> seconds of silence, then closed (a long probe for the port-collision check)
                   no file → nomatch. Every start and every request is logged to <g5>/meshmud-calls.log.
  --role presence  current/share/meshtastic-presence/meshtastic_presence.py (behind venvs/presence/bin/python):
                   the reader's CLI (--endpoint host:port --out DIR [--once] [--timeout s]). --once: send
                   want_config, print {"schema":1,"my_node_info":…} and exit 0 on an answer, exit 3 when the
                   downlink closes without one. Without --once (the service reader after a match): connect, ask,
                   and idle.

<g5> is the directory named by --g5 (the wrappers devices.sh installs pass it).
"""
import argparse
import json
import os
import re
import socket
import sys
import threading
import time

START = b"\x94\xc3"


def frame(payload):
    return START + bytes([len(payload) >> 8, len(payload) & 0xFF]) + payload


def log(g5, line):
    try:
        with open(os.path.join(g5, "meshmud-calls.log"), "a") as f:
            f.write(line + "\n")
    except OSError:
        pass


def read_config(path):
    text = open(path).read(64 * 1024)
    dev = re.search(r'device\s*=\s*"([^"]+)"', text)
    addr = re.search(r'address\s*=\s*"([^"]+)"', text.split("[[downlink", 1)[-1])
    return (dev.group(1) if dev else ""), (addr.group(1) if addr else "127.0.0.1:11230")


def mode_for(g5, device):
    d = os.path.join(g5, "meshmud")
    try:
        names = sorted(os.listdir(d))
    except OSError:
        names = []
    for n in names:
        if n and n in device:
            lines = open(os.path.join(d, n)).read().split("\n")
            return lines[0].strip(), (lines[1].strip() if len(lines) > 1 else "")
    return "nomatch", ""


def serve(conn, g5, device):
    buf = b""
    try:
        while True:
            chunk = conn.recv(4096)
            if not chunk:
                return
            buf += chunk
            while len(buf) >= 4 and buf[:2] == START:
                n = (buf[2] << 8) | buf[3]
                if len(buf) < 4 + n:
                    break
                p, buf = buf[4:4 + n], buf[4 + n:]
                if p != b"want_config":
                    continue  # the Meshtastic client's protobuf frames: noise here
                mode, extra = mode_for(g5, device)
                log(g5, "want_config %s %s" % (device, mode))
                if mode == "match":
                    num = int(extra, 16) if re.fullmatch(r"[0-9a-f]{8}", extra or "") else 0x0A0A0A0A
                    info = {"my_node_info": {
                        "num": num, "id": "!%08x" % num, "long_name": "G5 bench board", "short_name": "G5",
                        "hw_model": "HELTEC_V3", "firmware_version": "2.7.11.abcdef0"}}
                    conn.sendall(frame(json.dumps(info).encode()))
                    continue
                if mode.startswith("slow:"):
                    time.sleep(float(mode.split(":", 1)[1] or 0))
                conn.close()
                return
            if len(buf) >= 4 and buf[:2] != START:
                i = buf.find(START)
                buf = buf[i:] if i >= 0 else b""
    except OSError:
        return


def meshmud(args):
    cfg = args.rest[args.rest.index("-config") + 1] if "-config" in args.rest else ""
    device, addr = read_config(cfg) if cfg else ("", "127.0.0.1:11230")
    log(args.g5, "start %s downlink %s pid %d" % (device, addr, os.getpid()))
    host, port = addr.rsplit(":", 1)
    srv = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    srv.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    srv.bind((host, int(port)))
    srv.listen(8)
    print("stub-meshmud: listening on %s for %s" % (addr, device), flush=True)
    while True:
        conn, _ = srv.accept()
        threading.Thread(target=serve, args=(conn, args.g5, device), daemon=True).start()


def presence(args):
    rest = args.rest

    def opt(name):
        return rest[rest.index(name) + 1] if name in rest and rest.index(name) + 1 < len(rest) else None

    once = "--once" in rest
    host, port = (opt("--endpoint") or "127.0.0.1:11230").rsplit(":", 1)
    timeout = float(opt("--timeout") or 60)
    try:
        s = socket.create_connection((host, int(port)), timeout=timeout)
    except OSError:
        sys.exit(1)
    s.sendall(frame(b"want_config"))
    buf = b""
    while True:
        try:
            chunk = s.recv(4096)
        except OSError:
            chunk = b""
        if not chunk:
            if once:
                sys.exit(3)
            time.sleep(1_000_000)
        buf += chunk
        i = buf.find(START)
        if i >= 0 and len(buf) >= i + 4:
            n = (buf[i + 2] << 8) | buf[i + 3]
            if len(buf) >= i + 4 + n:
                try:
                    msg = json.loads(buf[i + 4:i + 4 + n])
                except ValueError:
                    msg = {}
                buf = buf[i + 4 + n:]
                if once and "my_node_info" in msg:
                    print(json.dumps({"schema": 1, "my_node_info": msg["my_node_info"]}), flush=True)
                    sys.exit(0)


def main():
    # `--role R --g5 DIR` come first (the wrappers put them there); everything after is the caller's own argv
    # (meshmud's `start-daemon -config …`, the reader's `--endpoint … --out … --once …`), kept verbatim.
    argv = sys.argv[1:]
    if len(argv) < 4 or argv[0] != "--role" or argv[2] != "--g5" or argv[1] not in ("meshmud", "presence"):
        print("usage: stub-meshmud.py --role meshmud|presence --g5 DIR [ARGS…]", file=sys.stderr)
        sys.exit(2)
    args = argparse.Namespace(role=argv[1], g5=argv[3], rest=argv[4:])
    (meshmud if args.role == "meshmud" else presence)(args)


if __name__ == "__main__":
    main()
