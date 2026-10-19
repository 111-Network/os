# SPDX-License-Identifier: GPL-3.0-or-later
"""Integration test against the real meshtastic-python, with a fake Meshtastic node on a
loopback TCP socket. Skipped when meshtastic-python is not installed, so the plain unit run
needs nothing; run it inside the hash-locked presence venv:

    <venv>/bin/python -I -m unittest discover -s tests

It proves, against the pinned library, that (a) the only ToRadio messages that ever reach
the wire are one want_config_id per connection and the session keep-alive, a heartbeat with
nonce 0 (no heartbeat with another nonce, no disconnect, nothing else, including at close),
(b) the pubsub signatures we subscribe with are accepted, (c) the node DB, position and
telemetry events end up in the presence file, and (d) a keep-alive into a meshmud that stopped
reading times out instead of blocking, and the reader reconnects (Layer 5 review m5).
"""
import json
import os
import signal
import socket
import subprocess
import sys
import tempfile
import threading
import time
import unittest

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
sys.path.insert(0, ROOT)

import meshtastic_presence as mp  # noqa: E402

try:
    from meshtastic.protobuf import mesh_pb2, portnums_pb2, telemetry_pb2  # type: ignore[import-not-found]
    HAVE_LIB = True
except Exception:  # pragma: no cover - depends on the environment
    HAVE_LIB = False

START1, START2 = 0x94, 0xC3
MY_NUM = 0x0A0A0A0A
PEER = 0x1B2C3D4E


def frame(msg):
    body = msg.SerializeToString()
    return bytes([START1, START2, len(body) >> 8, len(body) & 0xFF]) + body


def wait_for(predicate, timeout=10.0):
    end = time.monotonic() + timeout
    while time.monotonic() < end:
        if predicate():
            return True
        time.sleep(0.02)
    return predicate()


class FakeMeshtasticNode(threading.Thread):
    """Speaks just enough of the stream API: answers want_config_id with a small node DB."""

    def __init__(self):
        super().__init__(daemon=True)
        self.srv = socket.socket()
        self.srv.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        self.srv.bind(("127.0.0.1", 0))
        self.srv.listen(4)
        self.port = self.srv.getsockname()[1]
        self.received = []  # variants of every ToRadio, in order, across connections
        self.heartbeats = []  # the nonce of every heartbeat, in order
        self.connections = []
        self.stalled = set()  # connections this node no longer reads from (a stuck meshmud)
        self.lock = threading.Lock()
        self.stop = False

    def run(self):
        self.srv.settimeout(0.2)
        while not self.stop:
            try:
                conn, _ = self.srv.accept()
            except socket.timeout:
                continue
            except OSError:
                return
            with self.lock:
                self.connections.append(conn)
            threading.Thread(target=self.serve, args=(conn,), daemon=True).start()

    def serve(self, conn):
        buf = b""
        conn.settimeout(0.2)
        while not self.stop:
            if conn in self.stalled:
                time.sleep(0.05)
                continue
            try:
                chunk = conn.recv(4096)
            except socket.timeout:
                continue
            except OSError:
                return
            if not chunk:
                return
            buf += chunk
            while True:
                i = buf.find(bytes([START1, START2]))
                if i < 0 or len(buf) < i + 4:
                    break
                n = (buf[i + 2] << 8) | buf[i + 3]
                if len(buf) < i + 4 + n:
                    break
                msg = mesh_pb2.ToRadio()
                msg.ParseFromString(buf[i + 4:i + 4 + n])
                buf = buf[i + 4 + n:]
                variant = msg.WhichOneof("payload_variant")
                with self.lock:
                    self.received.append(variant)
                    if variant == "heartbeat":
                        self.heartbeats.append(msg.heartbeat.nonce)
                if variant == "want_config_id":
                    self.send_config(conn, msg.want_config_id)

    def send_config(self, conn, config_id):
        out = []
        out.append(mesh_pb2.FromRadio(my_info=mesh_pb2.MyNodeInfo(my_node_num=MY_NUM, pio_env="heltec-v3")))
        out.append(mesh_pb2.FromRadio(metadata=mesh_pb2.DeviceMetadata(firmware_version="2.7.11.test")))
        me = mesh_pb2.NodeInfo(num=MY_NUM, user=mesh_pb2.User(id="!%08x" % MY_NUM, long_name="My Board",
                                                              short_name="MB"))
        out.append(mesh_pb2.FromRadio(node_info=me))
        peer = mesh_pb2.NodeInfo(
            num=PEER, snr=5.5, last_heard=1760000000, hops_away=1,
            user=mesh_pb2.User(id="!%08x" % PEER, long_name="Hill‮top\x07", short_name="HT",
                               public_key=bytes(range(32))),
            device_metrics=telemetry_pb2.DeviceMetrics(channel_utilization=7.25))
        out.append(mesh_pb2.FromRadio(node_info=peer))
        out.append(mesh_pb2.FromRadio(config_complete_id=config_id))
        for msg in out:
            conn.sendall(frame(msg))

    def push_packet(self, num, portnum, payload):
        pkt = mesh_pb2.MeshPacket()
        setattr(pkt, "from", num)
        pkt.to = 0xFFFFFFFF
        pkt.decoded.portnum = portnum
        pkt.decoded.payload = payload
        with self.lock:
            conns = list(self.connections)
        for conn in conns:
            try:
                conn.sendall(frame(mesh_pb2.FromRadio(packet=pkt)))
            except OSError:
                pass

    def drop_all(self):
        with self.lock:
            conns, self.connections = list(self.connections), []
        for conn in conns:
            try:
                conn.shutdown(socket.SHUT_RDWR)
            except OSError:
                pass
            conn.close()

    def close(self):
        self.stop = True
        self.drop_all()
        self.srv.close()


@unittest.skipUnless(HAVE_LIB, "meshtastic-python is not installed (run inside the presence venv)")
class RealLibraryTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.node = FakeMeshtasticNode()
        self.node.start()
        self.path = os.path.join(self.tmp.name, mp.OUT_NAME)

    def tearDown(self):
        self.node.close()
        self.tmp.cleanup()

    def doc(self):
        try:
            with open(self.path, "rb") as f:
                return json.loads(f.read())
        except (OSError, ValueError):
            return None

    def test_guard_classifies_real_protobuf_messages(self):
        self.assertTrue(mp.allowed_to_radio(mesh_pb2.ToRadio(want_config_id=7)))
        self.assertFalse(mp.allowed_to_radio(mesh_pb2.ToRadio(packet=mesh_pb2.MeshPacket())))
        self.assertFalse(mp.allowed_to_radio(mesh_pb2.ToRadio(disconnect=True)))
        hb = mesh_pb2.ToRadio()
        hb.heartbeat.SetInParent()  # nonce 0: the session keep-alive
        self.assertTrue(mp.allowed_to_radio(hb))
        self.assertTrue(mp.allowed_to_radio(mesh_pb2.ToRadio(heartbeat=mesh_pb2.Heartbeat(nonce=0))))
        for nonce in (1, 2, 69420, 2 ** 32 - 1):  # nonce 1 makes the board broadcast its NodeInfo
            self.assertFalse(mp.allowed_to_radio(mesh_pb2.ToRadio(heartbeat=mesh_pb2.Heartbeat(nonce=nonce))), nonce)
        self.assertFalse(mp.allowed_to_radio(mesh_pb2.ToRadio()))

    def test_library_surface_we_override_still_exists(self):
        from meshtastic.tcp_interface import TCPInterface
        for name in ("_sendToRadio", "_sendToRadioImpl", "_sendDisconnect", "_startHeartbeat", "_reconnect",
                     "myConnect", "_startConfig", "_waitConnected", "_disconnected", "connect", "_socket_shutdown",
                     "_writeBytes"):
            self.assertTrue(hasattr(TCPInterface, name), name)
        self.assertTrue(hasattr(mesh_pb2.ToRadio(), "heartbeat"))
        self.assertTrue(hasattr(mesh_pb2.Heartbeat(), "nonce"))

    def test_presence_file_events_and_exactly_one_want_config(self):
        reader = mp.PresenceReader("127.0.0.1", self.node.port, self.tmp.name, min_interval=0.0, poll=0.2,
                                   connect_timeout=10, backoff=mp.Backoff(0.2, 2.0, 1.0, 0.0))
        t = threading.Thread(target=reader.run, daemon=True)
        t.start()
        try:
            self.assertTrue(wait_for(lambda: self.doc() is not None), "no presence file")
            doc = self.doc()
            self.assertEqual(doc["schema"], 1)
            self.assertEqual([n["num"] for n in doc["nodes"]], [PEER])  # own node excluded
            row = doc["nodes"][0]
            self.assertEqual(row["id"], "!1b2c3d4e")
            self.assertEqual(row["long_name"], "Hilltop")  # bidi override and BEL stripped
            self.assertEqual(row["short_name"], "HT")
            self.assertEqual(row["snr"], 5.5)
            self.assertEqual(row["hops_away"], 1)
            self.assertEqual(row["last_heard_hint"], 1760000000)
            self.assertEqual(row["channel_utilization"], 7.25)
            self.assertEqual(len(row["pubkey"]), 44)

            pos = mesh_pb2.Position(latitude_i=515074000, longitude_i=-1278000, altitude=35)
            self.node.push_packet(PEER, portnums_pb2.PortNum.POSITION_APP, pos.SerializeToString())
            self.assertTrue(wait_for(lambda: "position" in self.doc()["nodes"][0]), "position event not applied")
            self.assertEqual(self.doc()["nodes"][0]["position"], {"lat": 51.5074, "lon": -0.1278, "alt": 35})

            tel = telemetry_pb2.Telemetry(device_metrics=telemetry_pb2.DeviceMetrics(channel_utilization=33.5))
            self.node.push_packet(PEER, portnums_pb2.PortNum.TELEMETRY_APP, tel.SerializeToString())
            self.assertTrue(wait_for(lambda: self.doc()["nodes"][0].get("channel_utilization") == 33.5),
                            "telemetry event not applied")

            # link loss: the reader reconnects (second want_config on a NEW connection)
            self.node.drop_all()
            self.assertTrue(wait_for(lambda: self.node.received.count("want_config_id") == 2, 15),
                            "no reconnect after link loss")
            self.assertTrue(wait_for(lambda: len(self.node.connections) == 1))
        finally:
            reader.stop()
            t.join(10)
        self.assertFalse(t.is_alive())
        time.sleep(0.5)  # a late disconnect/heartbeat, if there were one, would arrive now
        with self.node.lock:
            seen = list(self.node.received)
        self.assertEqual(seen, ["want_config_id", "want_config_id"],
                         "only want_config may reach the wire (the keep-alive is not due within this test)")

    def test_keepalive_heartbeat_reaches_the_wire_with_nonce_zero_and_nothing_else_does(self):
        reader = mp.PresenceReader("127.0.0.1", self.node.port, self.tmp.name, min_interval=0.0, poll=0.2,
                                   connect_timeout=10, backoff=mp.Backoff(0.2, 2.0, 1.0, 0.0),
                                   keepalive_interval=2.0)
        t = threading.Thread(target=reader.run, daemon=True)
        t.start()
        try:
            self.assertTrue(wait_for(lambda: self.doc() is not None), "no presence file")
            with self.node.lock:
                self.assertEqual(self.node.received, ["want_config_id"], "no heartbeat before the interval")
            self.assertTrue(wait_for(lambda: len(self.node.heartbeats) >= 2, 15), "no keep-alive heartbeats")
            self.assertGreaterEqual(reader.keepalives, 2)
            self.assertEqual(reader.keepalive_failures, 0)
        finally:
            reader.stop()
            t.join(10)
        self.assertFalse(t.is_alive())
        time.sleep(0.5)  # a late disconnect, if there were one, would arrive now
        with self.node.lock:
            seen, nonces = list(self.node.received), list(self.node.heartbeats)
        self.assertEqual(seen, ["want_config_id"] + ["heartbeat"] * len(nonces))
        self.assertTrue(nonces and all(n == 0 for n in nonces), nonces)

    def test_a_keepalive_into_a_meshmud_that_stopped_reading_times_out_and_the_reader_reconnects(self):
        reader = mp.PresenceReader("127.0.0.1", self.node.port, self.tmp.name, min_interval=0.0, poll=0.2,
                                   connect_timeout=10, backoff=mp.Backoff(0.2, 2.0, 1.0, 0.0),
                                   keepalive_interval=1.0,
                                   connector=lambda h, p, t: mp.default_connector(h, p, t, write_timeout=0.3))
        t = threading.Thread(target=reader.run, daemon=True)
        t.start()
        try:
            self.assertTrue(wait_for(lambda: self.doc() is not None), "no presence file")
            iface = reader._iface
            with self.node.lock:
                self.node.stalled.update(self.node.connections)  # meshmud stops reading
            sock = iface.socket
            quiet_since, started = None, time.monotonic()
            while time.monotonic() - started < 10:  # fill the path until the kernel takes nothing
                try:
                    sock.send(b"\0" * 65536, socket.MSG_DONTWAIT)
                    quiet_since = None
                except BlockingIOError:
                    quiet_since = quiet_since or time.monotonic()
                    if time.monotonic() - quiet_since >= 0.3:
                        break
                    time.sleep(0.02)
                except OSError:
                    break  # the keep-alive already timed out and closed it
            self.assertTrue(wait_for(lambda: reader.keepalive_failures >= 1, 15), "the keep-alive never timed out")
            self.assertTrue(wait_for(lambda: self.node.received.count("want_config_id") >= 2, 15),
                            "no reconnect after the keep-alive timed out")
            self.assertIsNot(reader._iface, iface)
        finally:
            reader.stop()
            t.join(10)
        self.assertFalse(t.is_alive(), "the reader hung (close waiting on a reader thread stuck in recv?)")
        with self.node.lock:
            self.assertTrue(all(n == 0 for n in self.node.heartbeats), self.node.heartbeats)
            self.assertTrue(set(self.node.received) <= {"want_config_id", "heartbeat"}, self.node.received)

    def test_the_guard_refuses_heartbeat_nonce_one_and_every_other_toradio_on_the_real_interface(self):
        iface = mp.default_connector("127.0.0.1", self.node.port, 10)
        try:
            self.assertTrue(iface.session_keepalive())
            self.assertTrue(wait_for(lambda: self.node.heartbeats == [0]))
            refused = [
                mesh_pb2.ToRadio(heartbeat=mesh_pb2.Heartbeat(nonce=1)),
                mesh_pb2.ToRadio(heartbeat=mesh_pb2.Heartbeat(nonce=2)),
                mesh_pb2.ToRadio(disconnect=True),
                mesh_pb2.ToRadio(want_config_id=99),  # a second want_config
                mesh_pb2.ToRadio(packet=mesh_pb2.MeshPacket(to=0xFFFFFFFF)),
            ]
            for msg in refused:
                iface._sendToRadio(msg)
                iface._sendToRadioImpl(msg)  # and the implementation layer, called directly
            self.assertEqual(iface.dropped_to_radio, 2 * len(refused))
            self.assertTrue(iface.session_keepalive())  # the guard is still open for the real keep-alive
            self.assertTrue(wait_for(lambda: self.node.heartbeats == [0, 0]))
            time.sleep(0.5)
        finally:
            iface.close()
        time.sleep(0.3)
        with self.node.lock:
            self.assertEqual(self.node.received, ["want_config_id", "heartbeat", "heartbeat"])
            self.assertEqual(self.node.heartbeats, [0, 0])

    def test_once_prints_my_node_info_and_sends_only_want_config(self):
        code, line = mp.run_once("127.0.0.1", self.node.port, 10)
        self.assertEqual(code, 0, line)
        info = json.loads(line)["my_node_info"]
        self.assertEqual(info["num"], MY_NUM)
        self.assertEqual(info["id"], "!0a0a0a0a")
        self.assertEqual(info["long_name"], "My Board")
        self.assertEqual(info["firmware_version"], "2.7.11.test")
        self.assertEqual(info["pio_env"], "heltec-v3")
        time.sleep(0.5)
        with self.node.lock:
            self.assertEqual(self.node.received, ["want_config_id"])

    def test_connect_refused_is_a_clean_failure(self):
        probe = socket.socket()
        probe.bind(("127.0.0.1", 0))
        port = probe.getsockname()[1]
        probe.close()  # nothing listens on it now
        code, line = mp.run_once("127.0.0.1", port, 3)
        self.assertEqual((code, line), (3, ""))

    def cli(self, *args):
        return subprocess.Popen([sys.executable, os.path.join(ROOT, "meshtastic_presence.py"), *args],
                                stdout=subprocess.PIPE, stderr=subprocess.PIPE)

    def test_cli_runs_until_sigterm_and_exits_zero(self):
        proc = self.cli("--endpoint", "127.0.0.1:%d" % self.node.port, "--out", self.tmp.name,
                        "--min-interval", "0", "--timeout", "10")
        try:
            self.assertTrue(wait_for(lambda: self.doc() is not None), "no presence file")
            proc.send_signal(signal.SIGTERM)
            self.assertEqual(proc.wait(timeout=10), 0)
        finally:
            if proc.poll() is None:
                proc.kill()
            proc.stdout.close()
            proc.stderr.close()
        time.sleep(0.3)
        with self.node.lock:
            self.assertEqual(self.node.received, ["want_config_id"])

    def test_cli_once_prints_one_json_line(self):
        proc = self.cli("--endpoint", "127.0.0.1:%d" % self.node.port, "--once", "--timeout", "10")
        out, err = proc.communicate(timeout=20)
        self.assertEqual(proc.returncode, 0, err)
        self.assertEqual(out.count(b"\n"), 1)
        self.assertEqual(json.loads(out)["my_node_info"]["short_name"], "MB")
        self.assertFalse(os.path.exists(self.path), "--once must not write the presence file")

    def test_sigterm_while_waiting_for_a_silent_node_is_prompt(self):
        silent = socket.socket()
        silent.bind(("127.0.0.1", 0))
        silent.listen(1)
        proc = self.cli("--endpoint", "127.0.0.1:%d" % silent.getsockname()[1], "--out", self.tmp.name,
                        "--timeout", "30")
        try:
            time.sleep(1.0)  # connected, waiting for a node database that never comes
            started = time.monotonic()
            proc.send_signal(signal.SIGTERM)
            self.assertEqual(proc.wait(timeout=10), 0)
            self.assertLess(time.monotonic() - started, 5.0)
        finally:
            if proc.poll() is None:
                proc.kill()
            proc.stdout.close()
            proc.stderr.close()
            silent.close()


if __name__ == "__main__":
    unittest.main()
