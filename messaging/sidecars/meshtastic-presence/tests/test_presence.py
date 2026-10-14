# SPDX-License-Identifier: GPL-3.0-or-later
"""Unit tests for meshtastic_presence (spec section 14): fake interface, no network,
no hardware, no meshtastic-python needed.

Run from the sidecar directory:  python3 -I -m pytest tests   (or: python3 -I -m unittest discover -s tests)
"""
import ast
import base64
import json
import os
import random
import re
import stat
import sys
import tempfile
import threading
import time
import unittest
from types import SimpleNamespace

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
sys.path.insert(0, ROOT)

import meshtastic_presence as mp  # noqa: E402

SOURCE_PATH = os.path.join(ROOT, "meshtastic_presence.py")

KEY = base64.b64encode(bytes(range(32))).decode("ascii")


def node(num, long_name="Alpha", short_name="ALP", **extra):
    d = {"num": num, "user": {"id": "!%08x" % num, "longName": long_name, "shortName": short_name}}
    d.update(extra)
    return d


class FakeInterface:
    """What the reader may touch: nodesByNum, myInfo, metadata, isConnected, close."""

    def __init__(self, nodes, my_num=0x0A0A0A0A):
        self.nodesByNum = nodes
        self.myInfo = SimpleNamespace(my_node_num=my_num, pio_env="heltec-v3")
        self.metadata = SimpleNamespace(firmware_version="2.7.11.abcdef0")
        self.isConnected = threading.Event()
        self.isConnected.set()
        self.closed = False
        self.touched = []

    def close(self):
        self.closed = True

    def __getattr__(self, name):  # only reached for attributes the fake does not define
        self.touched.append(name)
        raise AttributeError(name)


class FakeBus:
    def __init__(self):
        self.subs = {}

    def subscribe(self, listener, topic):
        self.subs.setdefault(topic, []).append(listener)

    def unsubscribe(self, listener, topic):
        if listener in self.subs.get(topic, []):
            self.subs[topic].remove(listener)

    def publish(self, topic, **kwargs):
        for listener in list(self.subs.get(topic, [])):
            listener(**kwargs)

    def count(self):
        return sum(len(v) for v in self.subs.values())


def setUpModule():
    import logging
    logging.disable(logging.CRITICAL)  # keep the expected warnings out of the test output


def tearDownModule():
    import logging
    logging.disable(logging.NOTSET)


def wait_for(predicate, timeout=5.0):
    end = time.monotonic() + timeout
    while time.monotonic() < end:
        if predicate():
            return True
        time.sleep(0.01)
    return predicate()


class SanitizeTests(unittest.TestCase):
    def test_control_characters_are_stripped(self):
        dirty = "A\x00B\x07C\nD\tE\x7fF\x85G H I"
        self.assertEqual(mp.sanitize_text(dirty), "ABCDEFGHI")

    def test_bidi_overrides_and_bom_are_stripped(self):
        self.assertEqual(mp.sanitize_text("ok‮evil‬⁦x⁩﻿‏"), "okevilx")

    def test_truncates_to_40_bytes_on_a_code_point_boundary(self):
        self.assertEqual(mp.sanitize_text("x" * 100), "x" * 40)
        two = mp.sanitize_text("é" * 30)  # 2 bytes each
        self.assertEqual(len(two.encode("utf-8")), 40)
        self.assertEqual(two, "é" * 20)
        four = mp.sanitize_text("\U0001F600" * 11)  # 4 bytes each, 44 total
        self.assertEqual(four, "\U0001F600" * 10)
        three = mp.sanitize_text("a" + "€" * 20)  # 1 + 3*13 = 40, a split would be 41..42
        self.assertLessEqual(len(three.encode("utf-8")), 40)
        three.encode("utf-8")  # valid UTF-8, no lone surrogate

    def test_surrogates_and_non_strings(self):
        self.assertEqual(mp.sanitize_text("a\ud800b"), "ab")
        for bad in (None, 5, 1.5, b"bytes", ["x"], {"a": 1}, True):
            self.assertEqual(mp.sanitize_text(bad), "")

    def test_trimmed(self):
        self.assertEqual(mp.sanitize_text("  hi  "), "hi")
        self.assertEqual(mp.sanitize_text("\x00 \x00"), "")


class NormalizeTests(unittest.TestCase):
    def full(self):
        return node(
            0xA1B2C3D4, "  Hill\x00top‮ ", "HT\n",
            user_extra=None,
            snr=6.75, hopsAway=2, lastHeard=1760000000,
            position={"latitudeI": 515074000, "longitudeI": -1278000, "altitude": 35},
            deviceMetrics={"channelUtilization": 12.3456, "batteryLevel": 80},
            lastReceived={"rxRssi": -97, "raw": object()},
        )

    def test_full_row_and_field_order(self):
        n = self.full()
        n["user"]["publicKey"] = KEY
        row = mp.normalize_node(0xA1B2C3D4, n)
        self.assertEqual(list(row), ["id", "num", "long_name", "short_name", "pubkey", "snr", "rssi",
                                     "hops_away", "last_heard_hint", "position", "channel_utilization"])
        self.assertEqual(row["id"], "!a1b2c3d4")
        self.assertEqual(row["num"], 0xA1B2C3D4)
        self.assertEqual(row["long_name"], "Hilltop")
        self.assertEqual(row["short_name"], "HT")
        self.assertEqual(row["pubkey"], KEY)
        self.assertEqual(row["snr"], 6.75)
        self.assertEqual(row["rssi"], -97)
        self.assertEqual(row["hops_away"], 2)
        self.assertEqual(row["last_heard_hint"], 1760000000)
        self.assertEqual(row["position"], {"lat": 51.5074, "lon": -0.1278, "alt": 35})
        self.assertEqual(row["channel_utilization"], 12.35)

    def test_id_is_rebuilt_from_the_number_never_taken_from_the_radio(self):
        n = node(0x00000042)
        n["user"]["id"] = "!deadbeef\n<script>"
        self.assertEqual(mp.normalize_node(0x42, n)["id"], "!00000042")

    def test_optional_fields_are_omitted_when_absent_or_out_of_range(self):
        row = mp.normalize_node(7, node(7))
        self.assertEqual(sorted(row), ["id", "long_name", "num", "short_name"])
        hostile = node(7, snr=1e9, hopsAway=9, lastHeard=-1,
                       position={"latitudeI": 910000000, "longitudeI": 0},
                       deviceMetrics={"channelUtilization": 150.0},
                       lastReceived={"rxRssi": 55})
        self.assertEqual(sorted(mp.normalize_node(7, hostile)), ["id", "long_name", "num", "short_name"])

    def test_non_finite_and_bool_values_are_refused(self):
        n = node(7, snr=float("nan"), hopsAway=True, lastHeard=True,
                 deviceMetrics={"channelUtilization": float("inf")}, lastReceived={"rxRssi": True})
        self.assertEqual(sorted(mp.normalize_node(7, n)), ["id", "long_name", "num", "short_name"])

    def test_position_variants(self):
        self.assertIsNone(mp.normalize_node(7, node(7, position={"latitudeI": 0, "longitudeI": 0})).get("position"))
        self.assertEqual(
            mp.normalize_node(7, node(7, position={"latitude": 1.5, "longitude": 2.5}))["position"],
            {"lat": 1.5, "lon": 2.5})
        self.assertNotIn("position", mp.normalize_node(7, node(7, position={"latitude": 1.5})))
        self.assertNotIn("alt", mp.normalize_node(7, node(7, position={"latitudeI": 10000000, "longitudeI": 10000000,
                                                                       "altitude": 10 ** 9}))["position"])

    def test_public_key_must_be_32_valid_base64_bytes(self):
        for bad in ("not base64!!", base64.b64encode(b"x" * 31).decode(), base64.b64encode(b"x" * 33).decode(),
                    "", None, 5, "A" * 65):
            n = node(7)
            n["user"]["publicKey"] = bad
            self.assertNotIn("pubkey", mp.normalize_node(7, n), bad)

    def test_bad_node_numbers(self):
        for bad in (0, -1, mp.BROADCAST_NUM, 2 ** 40, "7", 7.0, None, True):
            self.assertIsNone(mp.normalize_node(bad, node(7)), bad)

    def test_garbage_node_values_yield_empty_names_not_exceptions(self):
        for junk in ("garbage", 5, None, [1], {"user": "x"}, {"user": {"longName": 5}}):
            row = mp.normalize_node(9, junk)
            self.assertEqual((row["long_name"], row["short_name"]), ("", ""))

    def test_fuzz_never_raises_and_always_serializes(self):
        rnd = random.Random(1110)
        atoms = [None, True, False, 0, -1, 1, 7, 2 ** 31, 2 ** 40, 1.5, -2.5, float("nan"), float("inf"), "", "x",
                 "!1234", "\x00‮" * 3, "\U0001F600" * 30, KEY, b"b", [], {}, [1, 2]]

        def rand(depth=0):
            if depth > 2 or rnd.random() < 0.5:
                return rnd.choice(atoms)
            keys = ["user", "longName", "shortName", "publicKey", "position", "latitudeI", "longitudeI", "latitude",
                    "longitude", "altitude", "snr", "hopsAway", "lastHeard", "deviceMetrics", "channelUtilization",
                    "lastReceived", "rxRssi", "num", "id"]
            return {rnd.choice(keys): rand(depth + 1) for _ in range(rnd.randint(0, 6))}

        rows = []
        for i in range(3000):
            row = mp.normalize_node(rnd.choice([rnd.randint(1, 2 ** 32 - 2), rand()]), rand())
            if row is not None:
                rows.append(row)
        data, _ = mp.encode_document(rows)
        doc = json.loads(data)
        self.assertEqual(doc["schema"], 1)
        for row in doc["nodes"]:
            self.assertLessEqual(len(row["long_name"].encode("utf-8")), 40)
            self.assertLessEqual(len(row["short_name"].encode("utf-8")), 40)


class DocumentTests(unittest.TestCase):
    def test_empty_document(self):
        data, dropped = mp.encode_document([])
        self.assertEqual(data, b'{"schema":1,"nodes":[]}\n')
        self.assertEqual(dropped, 0)

    def test_schema_keys_and_numeric_order(self):
        rows = [mp.normalize_node(n, node(n)) for n in (30, 10, 20)]
        doc = json.loads(mp.encode_document(rows)[0])
        self.assertEqual(sorted(doc), ["nodes", "schema"])
        self.assertEqual([r["num"] for r in doc["nodes"]], [10, 20, 30])
        allowed = {"id", "num", "long_name", "short_name", "pubkey", "snr", "rssi", "hops_away", "last_heard_hint",
                   "position", "channel_utilization"}
        for r in doc["nodes"]:
            self.assertTrue(set(r) <= allowed)

    def test_deterministic(self):
        rows = [mp.normalize_node(n, node(n, lastHeard=100 + n)) for n in range(1, 50)]
        self.assertEqual(mp.encode_document(rows), mp.encode_document(list(reversed(rows))))

    def test_node_count_cap_drops_the_least_recently_heard(self):
        rows = [mp.normalize_node(n, node(n, long_name="", short_name="", lastHeard=n)) for n in range(1, 4300)]
        data, dropped = mp.encode_document(rows)
        doc = json.loads(data)
        self.assertEqual(len(doc["nodes"]), mp.MAX_NODES)
        self.assertEqual(dropped, len(rows) - mp.MAX_NODES)
        kept = {r["num"] for r in doc["nodes"]}
        self.assertIn(4299 - 1, kept)  # newest kept
        self.assertNotIn(1, kept)  # oldest dropped

    def test_byte_cap_holds_for_maximal_rows(self):
        big = "€" * 13  # 39 bytes of 3-byte characters
        rows = []
        for n in range(1, mp.MAX_NODES + 1):
            nd = node(n, big, big, snr=-12.34, hopsAway=3, lastHeard=2 ** 31 + n,
                      position={"latitudeI": -123456789, "longitudeI": -1234567890, "altitude": 12345},
                      deviceMetrics={"channelUtilization": 33.33}, lastReceived={"rxRssi": -120})
            nd["user"]["publicKey"] = KEY
            rows.append(mp.normalize_node(n, nd))
        full = len(json.dumps({"schema": 1, "nodes": rows}, ensure_ascii=False, separators=(",", ":")).encode())
        self.assertGreater(full, mp.MAX_FILE_BYTES, "the fixture must exceed the cap or the test proves nothing")
        data, dropped = mp.encode_document(rows)
        self.assertLessEqual(len(data), mp.MAX_FILE_BYTES)
        self.assertLess(len(data), 1 << 20)
        self.assertGreater(dropped, 0)
        self.assertEqual(len(json.loads(data)["nodes"]), len(rows) - dropped)


class AtomicWriteTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.dir = self.tmp.name

    def tearDown(self):
        self.tmp.cleanup()

    def test_write_replace_mode_and_no_leftovers(self):
        mp.write_atomic(self.dir, "meshtastic.json", b"one\n")
        mp.write_atomic(self.dir, "meshtastic.json", b"two\n")
        path = os.path.join(self.dir, "meshtastic.json")
        with open(path, "rb") as f:
            self.assertEqual(f.read(), b"two\n")
        self.assertEqual(stat.S_IMODE(os.stat(path).st_mode), 0o644)
        self.assertEqual(os.listdir(self.dir), ["meshtastic.json"])

    def test_symlink_at_the_target_name_is_replaced_not_followed(self):
        victim = os.path.join(self.dir, "victim")
        with open(victim, "w") as f:
            f.write("untouched")
        link = os.path.join(self.dir, "meshtastic.json")
        os.symlink(victim, link)
        mp.write_atomic(self.dir, "meshtastic.json", b"data\n")
        self.assertFalse(os.path.islink(link))
        with open(victim) as f:
            self.assertEqual(f.read(), "untouched")
        with open(link, "rb") as f:
            self.assertEqual(f.read(), b"data\n")

    def test_missing_directory_raises_and_leaves_nothing(self):
        with self.assertRaises(OSError):
            mp.write_atomic(os.path.join(self.dir, "nope"), "meshtastic.json", b"x")
        self.assertEqual(os.listdir(self.dir), [])


class MyNodeInfoTests(unittest.TestCase):
    def test_my_node_info(self):
        fake = FakeInterface({0x0A0A0A0A: node(0x0A0A0A0A, "Base\x00 Station", "BS", )})
        fake.nodesByNum[0x0A0A0A0A]["user"].update({"hwModel": "HELTEC_V3", "publicKey": KEY})
        info = mp.build_my_node_info(fake)
        self.assertEqual(info, {"num": 0x0A0A0A0A, "id": "!0a0a0a0a", "long_name": "Base Station", "short_name": "BS",
                                "hw_model": "HELTEC_V3", "firmware_version": "2.7.11.abcdef0",
                                "pio_env": "heltec-v3", "pubkey": KEY})

    def test_not_known_yet(self):
        fake = FakeInterface({})
        fake.myInfo = None
        self.assertIsNone(mp.build_my_node_info(fake))

    def test_run_once_prints_one_json_line(self):
        fake = FakeInterface({0x0A0A0A0A: node(0x0A0A0A0A)})
        code, line = mp.run_once("h", 1, 5, connector=lambda h, p, t: fake)
        self.assertEqual(code, 0)
        self.assertNotIn("\n", line)
        doc = json.loads(line)
        self.assertEqual(doc["schema"], 1)
        self.assertEqual(doc["my_node_info"]["num"], 0x0A0A0A0A)
        self.assertTrue(fake.closed)

    def test_run_once_failures_exit_3_with_no_stdout(self):
        def boom(h, p, t):
            raise OSError("refused")

        self.assertEqual(mp.run_once("h", 1, 5, connector=boom), (3, ""))
        fake = FakeInterface({})
        fake.myInfo = None
        self.assertEqual(mp.run_once("h", 1, 5, connector=lambda h, p, t: fake), (3, ""))
        self.assertTrue(fake.closed)


class BackoffAndEndpointTests(unittest.TestCase):
    def test_backoff_doubles_to_the_cap_and_resets(self):
        b = mp.Backoff(1.0, 2.0, 60.0, jitter=0.2, rng=lambda: 0.0)
        self.assertEqual([b.next() for _ in range(8)], [1, 2, 4, 8, 16, 32, 60, 60])
        b.reset()
        self.assertEqual(b.next(), 1)

    def test_jitter_only_shortens(self):
        b = mp.Backoff(1.0, 2.0, 60.0, jitter=0.2, rng=lambda: 1.0)
        waits = [b.next() for _ in range(10)]
        self.assertTrue(all(w <= 60.0 for w in waits))
        self.assertAlmostEqual(waits[0], 0.8)

    def test_parse_endpoint(self):
        self.assertEqual(mp.parse_endpoint("127.0.0.1:11230"), ("127.0.0.1", 11230))
        self.assertEqual(mp.parse_endpoint("[::1]:4403"), ("::1", 4403))
        self.assertEqual(mp.parse_endpoint("board.local:4403"), ("board.local", 4403))
        for bad in ("", "nohost", ":1", "h:", "h:0", "h:65536", "h:x", "a b:1", "h/x:1"):
            with self.assertRaises(ValueError, msg=bad):
                mp.parse_endpoint(bad)


class ReaderTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.out = self.tmp.name
        self.path = os.path.join(self.out, mp.OUT_NAME)
        self.bus = FakeBus()

    def tearDown(self):
        self.tmp.cleanup()

    def reader(self, connector=None, **kw):
        kw.setdefault("min_interval", 0.0)
        kw.setdefault("poll", 0.05)
        kw.setdefault("backoff", mp.Backoff(1.0, 2.0, 60.0, jitter=0.0))
        kw.setdefault("sleep", lambda s: None)
        return mp.PresenceReader("127.0.0.1", 11230, self.out, connector=connector, bus=self.bus, **kw)

    def load(self):
        with open(self.path, "rb") as f:
            return json.loads(f.read())

    def try_load(self):
        try:
            return self.load()
        except (OSError, ValueError):
            return None

    def test_session_writes_the_snapshot_without_the_own_node(self):
        fake = FakeInterface({0x0A0A0A0A: node(0x0A0A0A0A, "Me"), 5: node(5, "Five", lastHeard=50), 6: node(6, "Six")})
        r = self.reader()
        r.run_session(fake, max_steps=1)
        doc = self.load()
        self.assertEqual([n["num"] for n in doc["nodes"]], [5, 6])
        self.assertEqual(doc["nodes"][0]["long_name"], "Five")
        self.assertEqual(r.writes, 1)

    def test_established_with_no_other_nodes_still_produces_a_file(self):
        fake = FakeInterface({0x0A0A0A0A: node(0x0A0A0A0A)})
        self.reader().run_session(fake, max_steps=1)
        self.assertEqual(self.load(), {"schema": 1, "nodes": []})

    def test_events_trigger_a_rewrite_only_when_content_changed(self):
        fake = FakeInterface({5: node(5, "Five")})
        r = self.reader()
        r._subscribe()
        r.run_session(fake, max_steps=1)
        self.assertEqual(r.writes, 1)
        # event without a change: no rewrite
        self.bus.publish("meshtastic.node.updated", node={}, interface=fake)
        r.run_session(fake, max_steps=1)
        self.assertEqual(r.writes, 1)
        # a position packet arrives and the library updated its DB
        fake.nodesByNum[5]["position"] = {"latitudeI": 10000000, "longitudeI": 20000000}
        self.bus.publish("meshtastic.receive.position", packet={}, interface=fake)
        r.run_session(fake, max_steps=1)
        self.assertEqual(r.writes, 2)
        self.assertEqual(self.load()["nodes"][0]["position"], {"lat": 1.0, "lon": 2.0})
        # telemetry and user packets and a new node
        fake.nodesByNum[7] = node(7, "Seven", deviceMetrics={"channelUtilization": 5.0})
        self.bus.publish("meshtastic.receive.telemetry", packet={}, interface=fake)
        self.bus.publish("meshtastic.receive.user", packet={}, interface=fake)
        r.run_session(fake, max_steps=1)
        self.assertEqual([n["num"] for n in self.load()["nodes"]], [5, 7])
        r._unsubscribe()

    def test_subscribes_to_the_documented_topics_and_unsubscribes(self):
        r = self.reader()
        r._subscribe()
        self.assertEqual(set(self.bus.subs), {t for t, _ in mp.TOPICS})
        self.assertEqual(self.bus.count(), len(mp.TOPICS))
        r._unsubscribe()
        self.assertEqual(self.bus.count(), 0)

    def test_burst_writes_are_coalesced_by_min_interval(self):
        clock = [100.0]
        slept = []
        r = self.reader(min_interval=2.0, monotonic=lambda: clock[0], sleep=lambda s: slept.append(s))
        fake = FakeInterface({5: node(5)})
        r._subscribe()
        r.run_session(fake, max_steps=1)  # first write, no wait
        fake.nodesByNum[6] = node(6)
        clock[0] = 100.5
        self.bus.publish("meshtastic.node.updated", node={}, interface=fake)
        r.run_session(fake, max_steps=1)
        self.assertEqual(len(slept), 1)
        self.assertAlmostEqual(slept[0], 1.5)
        self.assertEqual(r.writes, 2)

    def test_the_reader_touches_nothing_but_the_read_surface(self):
        fake = FakeInterface({5: node(5)})
        r = self.reader()
        r._subscribe()
        r.run_session(fake, max_steps=2)
        self.bus.publish("meshtastic.connection.lost", interface=fake)
        r.run_session(fake, max_steps=2)
        mp.build_my_node_info(fake)
        self.assertEqual(fake.touched, [])

    def test_write_failure_is_counted_not_raised(self):
        r = mp.PresenceReader("h", 1, os.path.join(self.out, "missing"), bus=self.bus, min_interval=0.0,
                              poll=0.05, sleep=lambda s: None)
        self.assertFalse(r.write_snapshot(FakeInterface({5: node(5)})))
        self.assertEqual(r.write_errors, 1)

    def test_stale_lost_event_from_a_closed_interface_is_ignored(self):
        old, new = FakeInterface({}), FakeInterface({5: node(5)})
        r = self.reader()
        r._iface = new
        r.on_lost(interface=old)
        self.assertFalse(r._lost.is_set())
        r.on_lost(interface=new)
        self.assertTrue(r._lost.is_set())

    def test_run_removes_a_stale_file_then_connects_with_backoff(self):
        with open(self.path, "w") as f:
            f.write("{stale}")
        sleeps = []
        fake = FakeInterface({5: node(5)})
        calls = []

        def connector(host, port, timeout):
            calls.append((host, port))
            if len(calls) <= 2:
                raise OSError("refused")
            return fake

        r = self.reader(connector, sleep=lambda s: sleeps.append(s))
        t = threading.Thread(target=r.run, daemon=True)
        t.start()
        try:
            self.assertTrue(wait_for(lambda: self.try_load() is not None))  # "{stale}" does not parse
            self.assertEqual(self.load()["nodes"][0]["num"], 5)
            self.assertEqual(sleeps[:2], [1.0, 2.0])
            self.assertEqual(r.connect_failures, 2)
        finally:
            r.stop()
            t.join(5)
        self.assertFalse(t.is_alive())
        self.assertTrue(fake.closed)
        self.assertEqual(self.bus.count(), 0)

    def test_lost_link_closes_the_interface_and_reconnects_with_fresh_data(self):
        first, second = FakeInterface({5: node(5, "Old")}), FakeInterface({5: node(5, "New"), 6: node(6)})
        queue = [first, second]
        r = self.reader(lambda h, p, t: queue.pop(0))
        t = threading.Thread(target=r.run, daemon=True)
        t.start()
        try:
            self.assertTrue(wait_for(lambda: self.try_load() is not None))
            self.assertTrue(wait_for(lambda: self.load()["nodes"][0]["long_name"] == "Old"))
            self.bus.publish("meshtastic.connection.lost", interface=first)
            self.assertTrue(wait_for(lambda: first.closed))
            self.assertTrue(wait_for(lambda: len((self.try_load() or {}).get("nodes", [])) == 2))
            self.assertEqual(self.load()["nodes"][0]["long_name"], "New")
        finally:
            r.stop()
            t.join(5)
        self.assertTrue(second.closed)

    def test_silent_death_is_found_by_polling_isConnected(self):
        first, second = FakeInterface({5: node(5)}), FakeInterface({5: node(5), 6: node(6)})
        queue = [first, second]
        r = self.reader(lambda h, p, t: queue.pop(0))
        t = threading.Thread(target=r.run, daemon=True)
        t.start()
        try:
            self.assertTrue(wait_for(lambda: self.try_load() is not None))
            first.isConnected.clear()
            self.assertTrue(wait_for(lambda: first.closed))
            self.assertTrue(wait_for(lambda: len((self.try_load() or {}).get("nodes", [])) == 2))
        finally:
            r.stop()
            t.join(5)

    def test_stop_during_backoff_returns_promptly(self):
        r = mp.PresenceReader("h", 1, self.out, connector=lambda h, p, t: (_ for _ in ()).throw(OSError("x")),
                              bus=self.bus, backoff=mp.Backoff(30, 2, 60, 0.0))
        t = threading.Thread(target=r.run, daemon=True)
        t.start()
        self.assertTrue(wait_for(lambda: r.connect_failures >= 1))
        started = time.monotonic()
        r.stop()
        t.join(5)
        self.assertFalse(t.is_alive())
        self.assertLess(time.monotonic() - started, 2.0)


class CliTests(unittest.TestCase):
    def test_usage_errors_exit_2_without_touching_the_network(self):
        self.assertEqual(mp.main(["--endpoint", "nonsense", "--out", "."]), 2)
        self.assertEqual(mp.main(["--endpoint", "127.0.0.1:11230"]), 2)  # no --out
        self.assertEqual(mp.main(["--endpoint", "127.0.0.1:11230", "--out", "/nonexistent-dir-111"]), 2)

    def test_defaults(self):
        args = mp.build_parser().parse_args([])
        self.assertEqual(args.endpoint, "127.0.0.1:11230")
        self.assertFalse(args.once)


class NoSendPathTests(unittest.TestCase):
    """The reader has no send path of any kind (spec section 14). Grep-provable."""

    FORBIDDEN_WORDS = (
        "sendText", "sendData", "sendPacket", "_sendPacket", "sendAlert", "sendPosition", "sendTraceRoute",
        "sendTelemetry", "sendWaypoint", "deleteWaypoint", "sendHeartbeat", "sendMqttClientProxyMessage",
        "writeConfig", "writeChannel", "setOwner", "setChannel", "setFixedPosition", "setTime", "removeNode",
        "ignoreNode", "favoriteNode", "factoryReset", "resetNodeDb", "reboot", "shutdown", "localNode", "getNode",
        "requestChannels", "requestConfig", "beginSettingsTransaction", "commitSettingsTransaction",
        "sendMessage", "serial_interface", "ble_interface",
    )
    # `_sendToRadio` and `_sendToRadioImpl` are the two guarded overrides (both refuse everything but one want_config).

    def setUp(self):
        with open(SOURCE_PATH, encoding="utf-8") as f:
            self.source = f.read()
        self.tree = ast.parse(self.source)

    def test_no_send_or_write_api_name_appears_anywhere_in_the_source(self):
        for word in self.FORBIDDEN_WORDS:
            self.assertIsNone(re.search(r"\b" + re.escape(word) + r"\b", self.source), word)

    def test_only_the_two_override_names_contain_send(self):
        defs = {n.name for n in ast.walk(self.tree) if isinstance(n, ast.FunctionDef) and "send" in n.name.lower()}
        self.assertEqual(defs, {"_sendToRadio", "_sendToRadioImpl", "_sendDisconnect"})
        attrs = {n.attr for n in ast.walk(self.tree) if isinstance(n, ast.Attribute) and "send" in n.attr.lower()}
        self.assertEqual(attrs, {"_sendToRadio", "_sendToRadioImpl"})  # only the guards' own pass-through of want_config
        names = {n.id for n in ast.walk(self.tree) if isinstance(n, ast.Name) and "send" in n.id.lower()}
        self.assertEqual(names, set())

    def test_only_one_toradio_variant_is_allowed(self):
        self.assertEqual(mp.ALLOWED_TORADIO, frozenset({"want_config_id"}))

    def test_allowed_to_radio_with_fake_messages(self):
        class Msg:
            def __init__(self, variant):
                self.variant = variant

            def WhichOneof(self, group):  # noqa: N802
                assert group == "payload_variant"
                return self.variant

        self.assertTrue(mp.allowed_to_radio(Msg("want_config_id")))
        for variant in ("packet", "heartbeat", "disconnect", "mqttClientProxyMessage", "xmodemPacket", None):
            self.assertFalse(mp.allowed_to_radio(Msg(variant)), variant)
        self.assertFalse(mp.allowed_to_radio(object()))
        self.assertFalse(mp.allowed_to_radio(None))

    def test_module_imports_without_meshtastic_installed_or_loaded(self):
        # the library is imported lazily, only inside make_interface_class / PubsubBus
        top = [n for n in self.tree.body if isinstance(n, (ast.Import, ast.ImportFrom))]
        mods = set()
        for n in top:
            mods.update(a.name.split(".")[0] for a in n.names) if isinstance(n, ast.Import) else mods.add((n.module or "").split(".")[0])
        self.assertTrue(mods.isdisjoint({"meshtastic", "pubsub", "google", "serial", "bleak"}), mods)


if __name__ == "__main__":
    unittest.main()
