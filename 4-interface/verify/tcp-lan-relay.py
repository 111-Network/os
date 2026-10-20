#!/usr/bin/env python3
"""tcp-lan pass-through relay, frame reader and replayer for smoke.sh --three-node (stdlib only).

The Layer 5 `tcp-lan` wire (os/5-messaging/README.md §8.1): both sides send
hello = "111F" 0x01, then frames type(1) | len(2 BE) | body
(0x01 sealed · 0x02 announce · 0x03 keepalive · 0x04 card request · 0x05 chunk).

  relay   --listen IP:PORT --to IP:PORT --dir CAPDIR
          Accepts connections and pipes each one, byte for byte, to --to. Every
          connection n records what crossed it, raw, in CAPDIR/<n>.c2s (client to
          server) and CAPDIR/<n>.s2c, plus CAPDIR/<n>.peer (the client address).
          Nothing is parsed or changed on the way: the relay is a recording wire.
          Killing the process closes every connection (both ends see EOF).

  frames  FILE…
          One JSON object per file: {"file", "hello": bool, "frames": [[type, len], …],
          "trailing": bytes that do not make a whole frame}.

  replay  --capture CAPDIR/<n>.c2s --to IP:PORT [--card-request HEX] [--hold SECONDS]
          [--out FILE]
          Opens a FRESH socket to --to and sends what the captured client sent:
          its hello, its announce, every sealed frame (keepalives skipped), then an
          optional card request (0x04 + 32-byte NodeId). It reads whatever the server
          answers for --hold seconds and prints/writes one JSON summary:
          {"sent": {type: count}, "received_hello": bool, "received": [[type, len], …],
           "received_types": {type: count}, "closed_by_peer": bool}.
          §8.1 / §4: a replayed packet verifies but is never fresh, so the server must
          never attribute this socket — no sealed frame, no card answer, no push.
"""
import argparse
import json
import os
import socket
import sys
import threading
import time

HELLO = b"111F\x01"
TYPES = {1: "sealed", 2: "announce", 3: "keepalive", 4: "card_request", 5: "chunk"}


def addr(s):
    host, port = s.rsplit(":", 1)
    return host.strip("[]"), int(port)


def parse(data):
    """(hello_ok, [(type, len, body)], trailing) of one direction's raw bytes."""
    if data[:5] != HELLO:
        return False, [], len(data)
    i, out = 5, []
    while len(data) - i >= 3:
        t, n = data[i], (data[i + 1] << 8) | data[i + 2]
        if len(data) - i - 3 < n:
            break
        out.append((t, n, data[i + 3 : i + 3 + n]))
        i += 3 + n
    return True, out, len(data) - i


# ---------------------------------------------------------------- relay


def pipe(src, dst, path, done):
    try:
        with open(path, "ab", buffering=0) as f:
            while True:
                b = src.recv(65536)
                if not b:
                    break
                f.write(b)
                dst.sendall(b)
    except OSError:
        pass
    finally:
        done.set()


def relay(args):
    os.makedirs(args.dir, exist_ok=True)
    lh, lp = addr(args.listen)
    srv = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    srv.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    srv.bind((lh, lp))
    srv.listen(16)
    print("tcp-lan-relay: %s -> %s, capture %s" % (args.listen, args.to, args.dir), flush=True)
    n = 0
    while True:
        c, peer = srv.accept()
        n += 1
        try:
            u = socket.create_connection(addr(args.to), timeout=5)
            u.settimeout(None)
        except OSError as e:
            print("tcp-lan-relay: conn %d upstream refused: %s" % (n, e), flush=True)
            c.close()
            continue
        with open(os.path.join(args.dir, "%d.peer" % n), "w") as f:
            f.write("%s:%d\n" % peer)
        done = threading.Event()

        def run(c=c, u=u, n=n, done=done):
            a = threading.Thread(target=pipe, args=(c, u, os.path.join(args.dir, "%d.c2s" % n), done), daemon=True)
            b = threading.Thread(target=pipe, args=(u, c, os.path.join(args.dir, "%d.s2c" % n), done), daemon=True)
            a.start()
            b.start()
            done.wait()
            for s in (c, u):
                try:
                    s.shutdown(socket.SHUT_RDWR)
                except OSError:
                    pass
                s.close()
            print("tcp-lan-relay: conn %d closed" % n, flush=True)

        threading.Thread(target=run, daemon=True).start()
        print("tcp-lan-relay: conn %d from %s:%d" % (n, peer[0], peer[1]), flush=True)


# ---------------------------------------------------------------- frames


def frames(args):
    for path in args.files:
        with open(path, "rb") as f:
            ok, fr, trailing = parse(f.read())
        print(json.dumps({"file": path, "hello": ok, "frames": [[t, n] for t, n, _ in fr], "trailing": trailing}))


# ---------------------------------------------------------------- replay


def replay(args):
    with open(args.capture, "rb") as f:
        ok, fr, _ = parse(f.read())
    if not ok or not fr or fr[0][0] != 2:
        print(json.dumps({"error": "capture has no hello + announce"}))
        return 2
    out = bytearray(HELLO)
    sent = {}
    for t, n, body in fr:
        if t not in (1, 2):
            continue  # keepalives and reserved frames are not part of the replay
        out += bytes([t, n >> 8, n & 0xFF]) + body
        sent[TYPES[t]] = sent.get(TYPES[t], 0) + 1
    if args.card_request:
        nid = bytes.fromhex(args.card_request)
        assert len(nid) == 32, "card request NodeId must be 32 bytes"
        out += bytes([4, 0, 32]) + nid
        sent["card_request"] = 1
    s = socket.create_connection(addr(args.to), timeout=5)
    got = bytearray()
    closed = False
    stop = time.monotonic() + args.hold

    def reader():
        nonlocal closed
        while time.monotonic() < stop:
            s.settimeout(max(0.05, stop - time.monotonic()))
            try:
                b = s.recv(65536)
            except socket.timeout:
                continue
            except OSError:
                closed = True
                return
            if not b:
                closed = True
                return
            got.extend(b)

    th = threading.Thread(target=reader, daemon=True)
    th.start()
    try:
        s.sendall(bytes(out))
    except OSError:
        closed = True
    th.join()
    s.close()
    rok, rfr, _ = parse(bytes(got))
    types = {}
    for t, _, _ in rfr:
        k = TYPES.get(t, str(t))
        types[k] = types.get(k, 0) + 1
    summary = {
        "sent": sent,
        "received_hello": rok,
        "received": [[t, n] for t, n, _ in rfr],
        "received_types": types,
        "closed_by_peer": closed,
    }
    text = json.dumps(summary)
    if args.out:
        with open(args.out, "w") as f:
            f.write(text + "\n")
    print(text)
    return 0


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    r = sub.add_parser("relay")
    r.add_argument("--listen", required=True)
    r.add_argument("--to", required=True)
    r.add_argument("--dir", required=True)
    f = sub.add_parser("frames")
    f.add_argument("files", nargs="+")
    p = sub.add_parser("replay")
    p.add_argument("--capture", required=True)
    p.add_argument("--to", required=True)
    p.add_argument("--card-request")
    p.add_argument("--hold", type=float, default=3.0)
    p.add_argument("--out")
    args = ap.parse_args()
    return {"relay": relay, "frames": frames, "replay": replay}[args.cmd](args) or 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except KeyboardInterrupt:
        sys.exit(0)
