#!/usr/bin/env python3
"""G5's serial board stand-in: a pty whose far end answers like a MeshCore KISS modem, or not at all (stdlib only).

Modelled ONLY on the facts of MeshCore's MIT-licensed docs/kiss_modem_protocol.md as recorded in R05 §3.1 and in
1-bootstrap/crates/node111/src/daemon/devices/identify/meshcore.rs — never on Reticulum's RNodeInterface.py:
  - KISS framing: FEND 0xC0 … FEND; FESC 0xDB + TFEND 0xDC / TFESC 0xDD escapes; the type byte's low nibble is the
    command, the high nibble the port; frames up to 512 B unescaped;
  - host → modem SetHardware 0x06 + sub-command: GetVersion 0x11, GetIdentity 0x01;
  - modem → host the answer is `command | 0x80`: 0x06 0x91 <version> <reserved>, 0x06 0x81 <32-byte Ed25519 key>.

    meshcore-pty.py --link /dev/ttyUSB3 --state DIR

opens a pty, keeps its slave open, points --link (a symlink in the gate's private /dev) at the slave, writes
DIR/ready, then serves the master side. The mode is re-read from DIR/mode before every answer:
  meshcore      both answers (the key holds a FEND and a FESC byte: escaped on the wire)
  version-only  the Version answer only (a partial answer: identify must call it an Error, never NoMatch)
  silent        nothing at all (another firmware, or no board: NoMatch after the probe's timeout)
Every SetHardware sub-command asked is appended to DIR/asked (hex, one per line); any other frame's type byte to
DIR/other (rnsd driving the port as an RNode writes there; it is never answered). SIGTERM ends it.
"""
import argparse
import os
import pty
import select
import signal
import sys
import termios
import tty

FEND, FESC, TFEND, TFESC = 0xC0, 0xDB, 0xDC, 0xDD
SET_HARDWARE, GET_VERSION, GET_IDENTITY = 0x06, 0x11, 0x01
MAX_FRAME = 512


def escape(data):
    out = bytearray()
    for b in data:
        if b == FEND:
            out += bytes([FESC, TFEND])
        elif b == FESC:
            out += bytes([FESC, TFESC])
        else:
            out.append(b)
    return bytes(out)


def kiss(payload):
    return bytes([FEND]) + escape(payload) + bytes([FEND])


def append(path, line):
    with open(path, "a") as f:
        f.write(line + "\n")


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--link", required=True)
    ap.add_argument("--state", required=True)
    a = ap.parse_args()
    signal.signal(signal.SIGTERM, lambda *_: sys.exit(0))
    master, slave = pty.openpty()
    tty.setraw(slave, termios.TCSANOW)
    name = os.ttyname(slave)
    try:
        os.unlink(a.link)
    except FileNotFoundError:
        pass
    os.symlink(name, a.link)
    with open(os.path.join(a.state, "ready"), "w") as f:
        f.write(name + "\n")

    def mode():
        try:
            return open(os.path.join(a.state, "mode")).read().strip()
        except OSError:
            return "silent"

    frame, inside, esc = bytearray(), False, False
    while True:
        r, _, _ = select.select([master], [], [], 1.0)
        if not r:
            continue
        try:
            data = os.read(master, 4096)
        except OSError:
            continue
        for b in data:
            if b == FEND:
                if inside and frame:
                    handle(bytes(frame), master, a.state, mode())
                frame, inside, esc = bytearray(), True, False
            elif not inside:
                continue
            elif b == FESC:
                esc = True
            else:
                if esc:
                    b = {TFEND: FEND, TFESC: FESC}.get(b, b)
                    esc = False
                if len(frame) < MAX_FRAME:
                    frame.append(b)


def handle(frame, master, state, mode):
    cmd = frame[0] & 0x0F
    if cmd != SET_HARDWARE or len(frame) < 2:
        append(os.path.join(state, "other"), "%02x" % frame[0])
        return
    sub = frame[1]
    append(os.path.join(state, "asked"), "%02x" % sub)
    if mode == "silent":
        return
    if sub == GET_VERSION:
        os.write(master, kiss(bytes([SET_HARDWARE, GET_VERSION | 0x80, 3, 0])))
    elif sub == GET_IDENTITY and mode == "meshcore":
        key = bytes([FEND, FESC]) + bytes(range(2, 32))
        os.write(master, kiss(bytes([SET_HARDWARE, GET_IDENTITY | 0x80]) + key))


if __name__ == "__main__":
    main()
