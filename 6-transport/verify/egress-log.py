#!/usr/bin/env python3
# 6-transport/verify/egress-log.py — the egress logger of the Layer 6 gates (spec §15.10, §16 G3; R07 §5's shape).
#
# Two tools in one file, both test fixtures (never shipped, never run by 111d in production):
#
#   sniff  An AF_PACKET logger on one interface (tcpdump cannot drop privileges inside a user namespace, R07 §5).
#          Every frame seen on the interface, both directions, is classified:
#            peer        IPv4 between the two declared addresses (--allow A,B), IPv6 link-local unicast (fe80::/10)
#            link        link-scope only, never forwarded: ARP, IPv6 link-scope multicast (ff01/ff02, ff11/ff12; ND,
#                        MLD, the Reticulum AutoInterface discovery group), IPv4 224.0.0.0/24, broadcast
#            control     the positive control: UDP to --control-dst with a payload starting b"111-g3-control"
#            off_link    everything else (a packet that would leave the link): each one is logged as one JSON line
#                        with addresses, protocol, ports, a DNS query name and a TLS SNI when present
#          On SIGTERM/SIGINT it writes --summary (JSON: counts per class and direction) and exits 0.
#
#     python3 -I egress-log.py sniff --iface eth0 --allow 10.66.0.1,10.66.0.2 --out off.jsonl --summary sum.json
#
#   wrap   Runs a program under `strace -f -e trace=connect` (R07 §5) and taps its JSON Lines stdio: every line
#          111d writes to the program and every line it answers is copied through unchanged and appended to
#          --tap (`> ` toward the program, `< ` from it, `# start <pid> <monotonic s>` per run). It is the process
#          111d spawns, so its pid is the component's; SIGTERM/SIGINT are forwarded to the traced program itself
#          (strace would only detach), and strace runs with --kill-on-exit so nothing outlives the wrapper. Exits
#          with the program's exit code. With --inject DIR, a file DIR/<name>.line holding one protocol line is
#          written to the program's stdin between 111d's lines (tapped as `+ `) and renamed <name>.line.done: a
#          stand-in for a command 111d has no route for. G3 no longer uses it (since fix wave FA2 111d announces on
#          request itself, and lxmf.sh asserts that no `+ ` line exists); it stays for scripted-helper experiments.
#
#     python3 -I egress-log.py wrap --strace-log s.log --tap tap.log [--inject DIR] -- PROGRAM ARGS...
#
# Foreign text from the program's stdout lands in the tap file (scratch only, a test artifact); nothing here
# interprets it beyond copying bytes.
import argparse
import ipaddress
import json
import os
import signal
import socket
import struct
import subprocess
import sys
import threading
import time

ETH_P_ALL = 0x0003
PACKET_OUTGOING = 4
CONTROL_MAGIC = b"111-g3-control"


# --------------------------------------------------------------------------------------------- sniff
def dns_qname(payload):
    """The first question name of a DNS query, or None."""
    try:
        if len(payload) < 12:
            return None
        qd = struct.unpack("!H", payload[4:6])[0]
        if qd < 1:
            return None
        i, labels = 12, []
        while i < len(payload) and len(labels) < 64:
            n = payload[i]
            if n == 0:
                return ".".join(labels) or "."
            if n & 0xC0:
                return None
            labels.append(payload[i + 1:i + 1 + n].decode("ascii", "replace"))
            i += 1 + n
    except Exception:
        return None
    return None


def tls_sni(payload):
    """The SNI of a TLS ClientHello at the start of a TCP payload, or None (best effort)."""
    try:
        if len(payload) < 43 or payload[0] != 0x16 or payload[5] != 0x01:
            return None
        i = 9 + 2 + 32
        sid = payload[i]
        i += 1 + sid
        cs = struct.unpack("!H", payload[i:i + 2])[0]
        i += 2 + cs
        cm = payload[i]
        i += 1 + cm
        ext_end = i + 2 + struct.unpack("!H", payload[i:i + 2])[0]
        i += 2
        while i + 4 <= ext_end:
            et, el = struct.unpack("!HH", payload[i:i + 4])
            if et == 0 and el > 5:
                ln = struct.unpack("!H", payload[i + 7:i + 9])[0]
                return payload[i + 9:i + 9 + ln].decode("ascii", "replace")
            i += 4 + el
    except Exception:
        return None
    return None


def l4(proto, data, rec):
    """Ports and the DNS/SNI hints of a transport payload."""
    if proto in (6, 17) and len(data) >= 4:
        sport, dport = struct.unpack("!HH", data[:4])
        rec["sport"], rec["dport"] = sport, dport
        if proto == 17:
            body = data[8:]
            if 53 in (sport, dport):
                q = dns_qname(body)
                if q:
                    rec["dns"] = q
            return body
        off = (data[12] >> 4) * 4 if len(data) >= 13 else 20
        body = data[off:]
        sni = tls_sni(body)
        if sni:
            rec["sni"] = sni
        return body
    return b""


def classify(frame, allow, control_dst):
    """(class, record) of one Ethernet frame."""
    if len(frame) < 14:
        return "off_link", {"why": "runt frame", "len": len(frame)}
    etype = struct.unpack("!H", frame[12:14])[0]
    data = frame[14:]
    if etype == 0x8100 and len(frame) >= 18:
        etype = struct.unpack("!H", frame[16:18])[0]
        data = frame[18:]
    if etype == 0x0806:
        return "link", {"proto": "arp"}
    if etype == 0x0800 and len(data) >= 20:
        ihl = (data[0] & 0x0F) * 4
        proto = data[9]
        src = ipaddress.IPv4Address(data[12:16])
        dst = ipaddress.IPv4Address(data[16:20])
        rec = {"v": 4, "src": str(src), "dst": str(dst), "proto": proto, "len": len(data)}
        body = l4(proto, data[ihl:], rec)
        if control_dst is not None and dst == control_dst and proto == 17 and body.startswith(CONTROL_MAGIC):
            return "control", rec
        if src in allow and dst in allow:
            return "peer", rec
        if dst in ipaddress.IPv4Network("224.0.0.0/24") or dst == ipaddress.IPv4Address("255.255.255.255"):
            return "link", rec
        if any(dst == n.broadcast_address for n in allow_nets(allow)):
            return "link", rec
        return "off_link", rec
    if etype == 0x86DD and len(data) >= 40:
        nxt = data[6]
        src = ipaddress.IPv6Address(data[8:24])
        dst = ipaddress.IPv6Address(data[24:40])
        rec = {"v": 6, "src": str(src), "dst": str(dst), "proto": nxt, "len": len(data)}
        l4(nxt, data[40:], rec)
        if dst.is_multicast:
            scope = data[25] & 0x0F
            return ("link" if scope in (1, 2) else "off_link"), rec
        if dst.is_link_local and (src.is_link_local or src.is_unspecified):
            return "peer", rec
        return "off_link", rec
    return "off_link", {"ethertype": "0x%04x" % etype, "len": len(frame)}


_NETS = {}


def allow_nets(allow):
    key = tuple(sorted(str(a) for a in allow))
    if key not in _NETS:
        _NETS[key] = [ipaddress.IPv4Network("%s/24" % a, strict=False) for a in allow]
    return _NETS[key]


def sniff(args):
    allow = {ipaddress.IPv4Address(a) for a in args.allow.split(",") if a}
    control_dst = ipaddress.IPv4Address(args.control_dst) if args.control_dst else None
    s = socket.socket(socket.AF_PACKET, socket.SOCK_RAW, socket.htons(ETH_P_ALL))
    s.setsockopt(socket.SOL_SOCKET, socket.SO_RCVBUF, 4 * 1024 * 1024)
    s.bind((args.iface, 0))
    s.settimeout(0.5)
    counts = {}
    stop = threading.Event()

    def on_signal(*_):
        stop.set()

    signal.signal(signal.SIGTERM, on_signal)
    signal.signal(signal.SIGINT, on_signal)
    t0 = time.monotonic()
    out = open(args.out, "a", buffering=1)
    if args.ready:
        open(args.ready, "w").write("%d\n" % os.getpid())
    while not stop.is_set():
        try:
            frame, addr = s.recvfrom(65535)
        except socket.timeout:
            continue
        except InterruptedError:
            continue
        direction = "out" if addr[2] == PACKET_OUTGOING else "in"
        cls, rec = classify(frame, allow, control_dst)
        key = "%s_%s" % (cls, direction)
        counts[key] = counts.get(key, 0) + 1
        if cls in ("off_link", "control"):
            rec.update({"class": cls, "dir": direction, "iface": args.iface,
                        "t": round(time.monotonic() - t0, 3)})
            out.write(json.dumps(rec, sort_keys=True) + "\n")
    out.close()
    summary = {"iface": args.iface, "allow": sorted(str(a) for a in allow), "counts": counts,
               "seconds": round(time.monotonic() - t0, 1)}
    with open(args.summary + ".tmp", "w") as f:
        json.dump(summary, f, sort_keys=True)
    os.replace(args.summary + ".tmp", args.summary)
    return 0


# --------------------------------------------------------------------------------------------- wrap
def children_of(pid):
    out = []
    try:
        for tid in os.listdir("/proc/%d/task" % pid):
            with open("/proc/%d/task/%s/children" % (pid, tid)) as f:
                out.extend(int(x) for x in f.read().split())
    except OSError:
        pass
    return out


def wrap(args):
    cmd = args.cmd[1:] if args.cmd and args.cmd[0] == "--" else args.cmd
    if not cmd:
        sys.stderr.write("egress-log wrap: no program\n")
        return 2
    strace = ["strace", "-f", "-qq", "--kill-on-exit", "-e", "trace=connect", "-o", args.strace_log, "--"]

    def pdeathsig():
        try:
            import ctypes
            ctypes.CDLL(None, use_errno=True).prctl(1, signal.SIGKILL)  # PR_SET_PDEATHSIG
        except Exception:
            pass

    child = subprocess.Popen(strace + cmd, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                             preexec_fn=pdeathsig)
    tap = open(args.tap, "ab", buffering=0)
    tap_lock = threading.Lock()

    def note(prefix, line):
        with tap_lock:
            tap.write(prefix + line if line.endswith(b"\n") else prefix + line + b"\n")

    def forward(signum, _frame):
        # strace detaches on a signal instead of passing it on: signal the traced program itself
        for pid in children_of(child.pid):
            try:
                os.kill(pid, signum)
            except OSError:
                pass
        if not children_of(child.pid):
            try:
                child.send_signal(signum)
            except OSError:
                pass

    signal.signal(signal.SIGTERM, forward)
    signal.signal(signal.SIGINT, forward)

    in_lock = threading.Lock()

    def to_child(prefix, line):
        with in_lock:
            note(prefix, line)
            child.stdin.write(line if line.endswith(b"\n") else line + b"\n")
            child.stdin.flush()

    def pump_in():
        src = sys.stdin.buffer
        try:
            for line in iter(src.readline, b""):
                to_child(b"> ", line)
        except (BrokenPipeError, OSError, ValueError):
            pass
        try:
            with in_lock:
                child.stdin.close()
        except OSError:
            pass

    def inject():
        while child.poll() is None:
            try:
                names = sorted(n for n in os.listdir(args.inject) if n.endswith(".line"))
            except OSError:
                names = []
            for n in names:
                p = os.path.join(args.inject, n)
                try:
                    with open(p, "rb") as f:
                        line = f.read().strip()
                    os.replace(p, p + ".done")
                    if line:
                        to_child(b"+ ", line)
                except (OSError, ValueError):
                    pass
            time.sleep(0.2)

    note(b"# ", ("start %d %.3f" % (os.getpid(), time.monotonic())).encode())
    threading.Thread(target=pump_in, daemon=True).start()
    if args.inject:
        threading.Thread(target=inject, daemon=True).start()
    dst = sys.stdout.buffer
    try:
        for line in iter(child.stdout.readline, b""):
            note(b"< ", line)
            try:
                dst.write(line)
                dst.flush()
            except (BrokenPipeError, OSError):
                break
    finally:
        rc = child.wait()
        note(b"# ", ("exit %d" % rc).encode())
    return rc if rc >= 0 else 128 - rc


def main():
    ap = argparse.ArgumentParser(prog="egress-log.py")
    sub = ap.add_subparsers(dest="mode", required=True)
    s = sub.add_parser("sniff")
    s.add_argument("--iface", required=True)
    s.add_argument("--allow", required=True, help="the two peer IPv4 addresses, comma-separated")
    s.add_argument("--control-dst", default="192.0.2.1")
    s.add_argument("--out", required=True)
    s.add_argument("--summary", required=True)
    s.add_argument("--ready", help="write the pid here once the socket is bound")
    w = sub.add_parser("wrap")
    w.add_argument("--strace-log", required=True)
    w.add_argument("--tap", required=True)
    w.add_argument("--inject", help="a directory of <name>.line files to write to the program's stdin")
    w.add_argument("cmd", nargs=argparse.REMAINDER)
    args = ap.parse_args()
    return sniff(args) if args.mode == "sniff" else wrap(args)


if __name__ == "__main__":
    sys.exit(main())
