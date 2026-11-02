#!/usr/bin/env python3
# 6-transport/verify/lxmf/fake-board.py — G3's channel source: a stand-in for `meshmud` and the board behind it.
#
# G3 (spec §16) needs a relayed copy into an LXMF recipient list, and an edge needs a channel source (§8: LXMF has
# no channels). The only channel networks a real 111d runs are Meshtastic and the app peers; this fixture is the
# smallest real one: `current/bin/meshmud` in node A's root execs it, so 111d's own Meshtastic client (B3) attaches
# to it exactly as to meshmud's downlink — the 0x94 0xC3 <len hi> <len lo> framing with protobuf `FromRadio` /
# `ToRadio` frames, hand-encoded here from the field numbers of the protobuf snapshot the `meshtastic` 0.1.9 crate
# vendors (no protobuf library, no Meshtastic code). A test fixture, never shipped.
#
#   python3 -I fake-board.py --ctl DIR [meshmud's own args are ignored]
#
# - Downlink: TCP 127.0.0.1:11230 (meshmud's, `N/meshtastic/meshmud.rs`). Each `want_config_id` is answered with a
#   full config stream: MyNodeInfo, the board's own NodeInfo (not licensed), the primary channel 0 on the default
#   key (psk 0x01, no uplink), LoRa config (EU_868, LONG_FAST), the node DB (one peer "Relayer"), then
#   `config_complete_id` with the client's id. A text the client sends is answered with QueueStatus res 0.
# - Uplink: 111d counts the board present while an ESTABLISHED TCP socket to the binding's uplink exists, so this
#   holds one to itself on 127.0.0.1:4403 (the binding is `tcp://127.0.0.1:4403`).
# - Control: a file `<ctl>/text-<n>` makes the peer "Relayer" post its contents on channel 0 to every connected
#   client (one MeshPacket, TEXT_MESSAGE_APP, a fresh packet id); the file is renamed `<ctl>/text-<n>.sent`.
#   `<ctl>/board.log` records what was served.
import argparse
import os
import socket
import struct
import sys
import threading
import time

DOWNLINK = ("127.0.0.1", 11230)
UPLINK = ("127.0.0.1", 4403)
MY_NUM = 0x0A0A0A0A        # the bound board (params.node_num 168430090)
PEER_NUM = 0x0B0B0B0B      # the channel member whose posts G3 relays
BCAST = 0xFFFFFFFF
REGION_EU_868 = 3
TEXT_MESSAGE_APP = 1
ROLE_PRIMARY = 1


# ------------------------------------------------------------------------------------- protobuf encoding
def varint(n):
    out = bytearray()
    n &= (1 << 64) - 1
    while True:
        b = n & 0x7F
        n >>= 7
        if n:
            out.append(b | 0x80)
        else:
            out.append(b)
            return bytes(out)


def f_varint(tag, n):
    return varint(tag << 3) + varint(n)


def f_bytes(tag, b):
    if isinstance(b, str):
        b = b.encode("utf-8")
    return varint((tag << 3) | 2) + varint(len(b)) + b


def f_fixed32(tag, n):
    return varint((tag << 3) | 5) + struct.pack("<I", n & 0xFFFFFFFF)


def framed(payload):
    return bytes([0x94, 0xC3, (len(payload) >> 8) & 0xFF, len(payload) & 0xFF]) + payload


def from_radio(tag, body, wire="msg"):
    """FromRadio { id = 1; oneof payload_variant { packet = 2; my_info = 3; node_info = 4; config = 5;
    config_complete_id = 7; channel = 10; queueStatus = 11 } }."""
    if wire == "varint":
        return framed(f_varint(tag, body))
    return framed(f_bytes(tag, body))


def user(num, long_name, short_name, licensed=False):
    # User { id = 1; long_name = 2; short_name = 3; hw_model = 5; is_licensed = 6 }
    b = f_bytes(1, "!%08x" % num) + f_bytes(2, long_name) + f_bytes(3, short_name) + f_varint(5, 43)
    if licensed:
        b += f_varint(6, 1)
    return b


def node_info(num, long_name, short_name, licensed=False):
    # NodeInfo { num = 1; user = 2; last_heard = 5 (fixed32) }
    return from_radio(4, f_varint(1, num) + f_bytes(2, user(num, long_name, short_name, licensed)))


def config_stream(want_id):
    out = bytearray()
    out += from_radio(3, f_varint(1, MY_NUM))                                   # MyNodeInfo.my_node_num
    out += node_info(MY_NUM, "G3 board", "G3")
    # Channel { index = 1; settings = 2 { psk = 2; name = 3; uplink_enabled = 5 }; role = 3 }
    settings = f_bytes(2, b"\x01")
    out += from_radio(10, f_varint(1, 0) + f_bytes(2, settings) + f_varint(3, ROLE_PRIMARY))
    # Config { lora = 6 { use_preset = 1; modem_preset = 2; region = 7; hop_limit = 8; tx_enabled = 9 } }
    lora = f_varint(1, 1) + f_varint(2, 0) + f_varint(7, REGION_EU_868) + f_varint(8, 3) + f_varint(9, 1)
    out += from_radio(5, f_bytes(6, lora))
    out += node_info(PEER_NUM, "Relayer", "RL")
    out += from_radio(7, want_id, wire="varint")                                 # config_complete_id
    return bytes(out)


def text_packet(packet_id, text):
    # MeshPacket { from = 1 (fixed32); to = 2 (fixed32); channel = 3; decoded = 4 { portnum = 1; payload = 2 };
    #              id = 6 (fixed32); rx_time = 7 (fixed32); hop_limit = 9; rx_rssi = 12; hop_start = 15 }
    data = f_varint(1, TEXT_MESSAGE_APP) + f_bytes(2, text.encode("utf-8"))
    p = (f_fixed32(1, PEER_NUM) + f_fixed32(2, BCAST) + f_varint(3, 0) + f_bytes(4, data) + f_fixed32(6, packet_id)
         + f_varint(9, 3) + f_varint(12, (1 << 64) - 90) + f_varint(15, 3))
    return from_radio(2, p)


def queue_status(packet_id):
    # QueueStatus { res = 1; free = 2; maxlen = 3; mesh_packet_id = 4 }
    return from_radio(11, f_varint(1, 0) + f_varint(2, 15) + f_varint(3, 16) + f_varint(4, packet_id))


# ------------------------------------------------------------------------------------- protobuf decoding
def fields(buf):
    """(tag, wire, value) of one message's top-level fields; raises on a malformed buffer."""
    i, out = 0, []
    while i < len(buf):
        key, i = read_varint(buf, i)
        tag, wire = key >> 3, key & 7
        if wire == 0:
            v, i = read_varint(buf, i)
        elif wire == 2:
            n, i = read_varint(buf, i)
            v, i = buf[i:i + n], i + n
        elif wire == 5:
            v, i = struct.unpack("<I", buf[i:i + 4])[0], i + 4
        elif wire == 1:
            v, i = buf[i:i + 8], i + 8
        else:
            raise ValueError("wire type %d" % wire)
        out.append((tag, wire, v))
    return out


def read_varint(buf, i):
    shift = n = 0
    while True:
        b = buf[i]
        i += 1
        n |= (b & 0x7F) << shift
        if not b & 0x80:
            return n, i
        shift += 7
        if shift > 63:
            raise ValueError("varint")


# ------------------------------------------------------------------------------------- the board
class Board:
    def __init__(self, ctl):
        self.ctl = ctl
        self.lock = threading.Lock()
        self.clients = []
        self.next_id = 0x51000000
        self.log = open(os.path.join(ctl, "board.log"), "a", buffering=1)

    def say(self, text):
        self.log.write("%.3f %s\n" % (time.monotonic(), text))

    def send_all(self, data):
        with self.lock:
            for c in list(self.clients):
                try:
                    c.sendall(data)
                except OSError:
                    self.clients.remove(c)

    def serve(self, conn):
        with self.lock:
            self.clients.append(conn)
        buf = b""
        try:
            while True:
                chunk = conn.recv(4096)
                if not chunk:
                    break
                buf += chunk
                while True:
                    j = buf.find(b"\x94\xc3")
                    if j < 0:
                        buf = buf[-1:]
                        break
                    if len(buf) < j + 4:
                        buf = buf[j:]
                        break
                    n = (buf[j + 2] << 8) | buf[j + 3]
                    if len(buf) < j + 4 + n:
                        buf = buf[j:]
                        break
                    payload, buf = buf[j + 4:j + 4 + n], buf[j + 4 + n:]
                    self.to_radio(conn, payload)
        except OSError:
            pass
        finally:
            with self.lock:
                if conn in self.clients:
                    self.clients.remove(conn)
            conn.close()

    def to_radio(self, conn, payload):
        try:
            fs = fields(payload)
        except (ValueError, IndexError, struct.error):
            self.say("undecodable ToRadio (%d B)" % len(payload))
            return
        for tag, wire, v in fs:
            if tag == 3 and wire == 0:                     # want_config_id
                self.say("want_config_id %d: config stream served" % v)
                conn.sendall(config_stream(v))
            elif tag == 1 and wire == 2:                   # packet (a send from the client)
                pid = 0
                try:
                    for t, w, x in fields(v):
                        if t == 6 and w == 5:
                            pid = x
                except (ValueError, IndexError, struct.error):
                    pass
                self.say("packet from the client (id %d): queued" % pid)
                conn.sendall(queue_status(pid))
            elif tag == 7:
                pass                                       # heartbeat
            else:
                self.say("ToRadio field %d ignored" % tag)

    def control(self):
        while True:
            try:
                names = sorted(n for n in os.listdir(self.ctl) if n.startswith("text-") and not n.endswith(".sent"))
            except OSError:
                names = []
            for n in names:
                p = os.path.join(self.ctl, n)
                try:
                    text = open(p, encoding="utf-8").read().strip()
                except OSError:
                    continue
                self.next_id += 1
                self.send_all(text_packet(self.next_id, text))
                self.say("posted %s on channel 0 as packet %d to %d client(s)" % (n, self.next_id, len(self.clients)))
                os.replace(p, p + ".sent")
            time.sleep(0.2)


def listener(addr):
    s = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    s.bind(addr)
    s.listen(8)
    return s


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--ctl", required=True)
    args, _rest = ap.parse_known_args()
    os.makedirs(args.ctl, exist_ok=True)
    board = Board(args.ctl)
    up = listener(UPLINK)
    held = socket.create_connection(UPLINK)          # meshmud's uplink to the board: ESTABLISHED while we run
    up_conn, _ = up.accept()
    down = listener(DOWNLINK)
    board.say("serving downlink %s:%d, uplink %s:%d held" % (DOWNLINK + UPLINK))
    threading.Thread(target=board.control, daemon=True).start()
    try:
        while True:
            conn, _ = down.accept()
            threading.Thread(target=board.serve, args=(conn,), daemon=True).start()
    finally:
        held.close()
        up_conn.close()


if __name__ == "__main__":
    sys.exit(main())
