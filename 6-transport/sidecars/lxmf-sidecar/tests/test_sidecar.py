"""lxmf-sidecar offline tests (spec section 6): protocol framing, every command, the state mapping, the
monotone rule, parking, the relay marker, attachment stripping, caps, announce timing, exit codes.

A fake RNS + LXMF surface is injected through sys.modules (tests/_support.py); no network, no hardware.
"""
import ast
import io
import json
import os
import stat
import threading
import unittest

import _support as t
from _support import World, inbound

sc = t.sc

FOREIGN = "FOREIGN-TEXT-7f3a"


def lib(w):
    return w.helper.lib


class Framing(unittest.TestCase):
    def setUp(self):
        self.w = World(start=False)
        self.addCleanup(self.w.close)

    def test_one_object_per_line_flushed(self):
        out = self.w.out
        self.assertTrue(out.emit({"type": "x", "text": "a\nb\r\nc   é中"}))
        self.assertTrue(out.emit({"type": "y"}))
        raw = self.w.stdout.getvalue()
        self.assertEqual(raw.count(b"\n"), 2)
        self.assertEqual(self.w.stdout.flushes, 2)
        objs = self.w.stdout.objs()
        self.assertEqual(objs[0]["text"], "a\nb\r\nc   é中")
        # UTF-8, not \u escapes, for readable lines
        self.assertIn("é".encode("utf-8"), raw)

    def test_oversize_line_refused(self):
        self.assertFalse(self.w.out.emit({"type": "x", "text": "a" * sc.MAX_LINE}))
        self.assertEqual(self.w.stdout.getvalue(), b"")
        self.assertEqual(self.w.out.refused, 1)
        self.assertIn("oversize", self.w.stderr.getvalue())
        # the limit is on the line: exactly MAX_LINE bytes passes
        pad = sc.MAX_LINE - len(sc.Emitter.encode({"type": "x", "text": ""}))
        self.assertTrue(self.w.out.emit({"type": "x", "text": "a" * pad}))
        self.assertEqual(len(self.w.stdout.lines()[0].encode("utf-8")), sc.MAX_LINE)

    def test_unserialisable_refused(self):
        self.assertFalse(self.w.out.emit({"type": "x", "v": float("nan")}))
        self.assertFalse(self.w.out.emit({"type": "x", "v": object()}))
        self.assertEqual(self.w.stdout.getvalue(), b"")

    def test_broken_pipe_closes_emitter(self):
        class Broken:
            def write(self, b):
                raise BrokenPipeError()

            def flush(self):
                pass

        out = sc.Emitter(Broken(), self.w.log)
        self.assertFalse(out.emit({"type": "x"}))
        self.assertTrue(out.closed)
        self.assertFalse(out.emit({"type": "y"}))

    def test_reader_oversize_drained_and_next_line_survives(self):
        big = b"x" * (sc.MAX_LINE * 2 + 17)
        data = b'{"type":"announce"}\n' + big + b"\n" + b'{"type":"quit"}\n'
        got = []
        sc.read_commands(io.BytesIO(data), got.append)
        self.assertEqual([k for k, _ in got], ["line", "bad", "line", "eof"])
        self.assertEqual(got[1][1], "oversize")

    def test_reader_exact_limit_line_is_accepted(self):
        line = b"y" * sc.MAX_LINE
        got = []
        sc.read_commands(io.BytesIO(line + b"\n"), got.append)
        self.assertEqual([k for k, _ in got], ["line", "eof"])
        got = []
        sc.read_commands(io.BytesIO(b"y" * (sc.MAX_LINE + 1) + b"\n"), got.append)
        self.assertEqual([k for k, _ in got], ["bad", "eof"])

    def test_reader_unterminated_last_line_and_blank_lines(self):
        got = []
        sc.read_commands(io.BytesIO(b'\n\n{"type":"quit"}'), got.append)
        self.assertEqual([k for k, _ in got], ["line", "eof"])


class Malformed(unittest.TestCase):
    def setUp(self):
        self.w = World()
        self.addCleanup(self.w.close)

    def test_malformed_lines_counted_never_crash(self):
        bad = [b"not json", b"[]", b"42", b'"str"', b"{}", b'{"type":"nope"}', b"\xff\xfe\n",
               b'{"type":"send"}',
               b'{"type":"send","id":"","to":"%s","text":"x"}' % (b"a" * 32),
               b'{"type":"send","id":"i","to":"zz","text":"x"}',
               b'{"type":"send","id":"i","to":"%s","text":5}' % (b"a" * 32),
               b'{"type":"send","id":"i","to":"%s","text":"x","fields":[1]}' % (b"a" * 32),
               b'{"type":"send","id":"i","to":"%s","text":"x","fields":{"abc":1}}' % (b"a" * 32),
               b'{"type":"send","id":"i","to":"%s","text":"x","fields":{"relay":{"v":1}}}' % (b"a" * 32),
               b'{"type":"name"}', b'{"type":"name","display":""}',
               b'{"type":"name","display":"\\u0000\\u0007"}',
               b"[" * 5000]
        before = len(self.w.stdout.objs())
        for line in bad:
            self.assertTrue(self.w.helper.handle_line(line + b"\n"), line[:30])
        self.assertEqual(self.w.helper.malformed, len(bad))
        self.assertEqual(len(self.w.stdout.objs()), before)  # nothing leaks to the protocol
        warns = [l for l in self.w.stderr.getvalue().splitlines() if "malformed" in l or "bad fields" in l]
        self.assertEqual(len(warns), len(bad))  # exactly one stderr line each
        self.assertEqual(self.w.router.outbound, [])

    def test_foreign_text_not_echoed_to_stderr(self):
        self.w.helper.handle_line(('{"type":"nope","x":"%s"}\n' % FOREIGN).encode())
        self.w.helper.handle_line(('{"type":"send","id":"%s","to":"zz","text":"%s"}\n' % (FOREIGN, FOREIGN)).encode())
        self.assertNotIn(FOREIGN, self.w.stderr.getvalue())

    def test_commands_ignore_extra_keys(self):
        self.assertTrue(self.w.cmd({"type": "announce", "extra": 1}))
        self.assertEqual(self.w.helper.malformed, 0)

    def test_quit_stops(self):
        self.assertFalse(self.w.cmd({"type": "quit"}))


class StartAndCommands(unittest.TestCase):
    def setUp(self):
        self.w = World()
        self.addCleanup(self.w.close)

    def test_ready_is_first_line_with_dest_pubkey_instance(self):
        first = self.w.stdout.objs()[0]
        self.assertEqual(first["type"], "ready")
        self.assertEqual(first["dest"], self.w.router.dest.hash.hex())
        self.assertEqual(len(first["dest"]), 32)
        self.assertEqual(first["pubkey"], self.w.helper.identity.get_public_key().hex())
        self.assertEqual(len(first["pubkey"]), 128)
        self.assertEqual(first["instance"], "111")
        self.assertEqual(set(first), {"type", "dest", "pubkey", "instance"})

    def test_start_sequence_ready_link_then_one_announce(self):
        self.assertEqual([o["type"] for o in self.w.stdout.objs()], ["ready", "link"])
        self.assertEqual(self.w.of("link")[0]["state"], "up")
        self.assertEqual(len(self.w.router.announced), 1)

    def test_no_stamp_cost_advertised(self):
        self.assertIsNone(self.w.router.stamp_cost)

    def test_announce_command(self):
        n = len(self.w.router.announced)
        self.w.cmd({"type": "announce"})
        self.assertEqual(len(self.w.router.announced), n + 1)

    def test_name_command_sets_name_and_announces(self):
        n = len(self.w.router.announced)
        self.w.cmd({"type": "name", "display": "My é Node\x07"})
        self.assertEqual(self.w.router.dest.display_name, "My é Node")
        self.assertEqual(len(self.w.router.announced), n + 1)
        self.assertEqual(self.w.router.announced[-1][1], "My é Node")

    def test_name_capped(self):
        self.w.cmd({"type": "name", "display": "é" * 500})
        self.assertLessEqual(len(self.w.router.dest.display_name.encode("utf-8")), sc.NAME_MAX_BYTES)

    def test_default_name_is_111_plus_four_hex(self):
        self.assertEqual(self.w.router.dest.display_name, "111-" + self.w.router.dest.hash.hex()[:4])

    def test_explicit_name(self):
        w = World(name="Hilltop")
        self.addCleanup(w.close)
        self.assertEqual(w.router.dest.display_name, "Hilltop")

    def test_router_storage_and_identity_modes(self):
        self.assertEqual(self.w.router.storagepath, self.w.args.storage)
        self.assertEqual(stat.S_IMODE(os.stat(self.w.args.storage).st_mode), 0o700)
        self.assertEqual(stat.S_IMODE(os.stat(self.w.args.identity).st_mode), 0o600)

    def test_identity_reused_across_runs(self):
        first = self.w.router.dest.hash
        self.w.fakes.uninstall()
        w2 = World.__new__(World)
        # restart over the same root
        w2.fakes = t.Fakes("client").install()
        w2.clock, w2.stdout, w2.stderr = t.FakeClock(), t.Rec(), io.StringIO()
        w2.args = self.w.args
        w2.log = sc.Log(w2.stderr, False)
        w2.out = sc.Emitter(w2.stdout, w2.log)
        w2.helper = sc.Helper(w2.args, w2.fakes.rns, w2.fakes.lxmf, w2.out, w2.log, w2.clock)
        w2.helper.attach()
        self.assertEqual(w2.helper.router.dest.hash, first)
        w2.fakes.uninstall()
        self.w.fakes.install()  # so the cleanup restores symmetrically

    def test_unreadable_identity_is_never_replaced(self):
        w = World(start=False)
        self.addCleanup(w.close)
        with open(w.args.identity, "wb") as f:
            f.write(b"short")
        with self.assertRaises(sc.StartupError):
            w.helper.attach()
        with open(w.args.identity, "rb") as f:
            self.assertEqual(f.read(), b"short")

    def test_rns_log_routed_to_stderr_ascii(self):
        call = self.w.fakes.reticulum_calls[0]
        call["logdest"]("café \x1b[31m red")
        line = [l for l in self.w.stderr.getvalue().splitlines() if "rns:" in l][0]
        self.assertTrue(all(" " <= c < "\x7f" for c in line))

    def test_reticulum_gets_the_client_config_dir_and_require_shared(self):
        call = self.w.fakes.reticulum_calls[0]
        self.assertEqual(call["configdir"], self.w.args.config_dir)
        self.assertTrue(call["require_shared_instance"])
        self.assertEqual(call["loglevel"], 2)


class ClientConfig(unittest.TestCase):
    def setUp(self):
        self.w = World(start=False, config="[reticulum]\n  share_instance = No\n  instance_name = 111\n"
                                           "  rpc_key = " + "ab" * 32 + "\n\n[interfaces]\n")
        self.addCleanup(self.w.close)
        self.path = os.path.join(self.w.args.config_dir, "config")

    def read(self):
        with open(self.path) as f:
            return f.read()

    def test_share_instance_refreshed_rpc_key_untouched(self):
        self.w.helper.attach()
        text = self.read()
        self.assertIn("share_instance = Yes", text)
        self.assertNotIn("share_instance = No", text)
        self.assertIn("rpc_key = " + "ab" * 32, text)
        self.assertEqual(stat.S_IMODE(os.stat(self.path).st_mode), 0o600)
        self.assertEqual(stat.S_IMODE(os.stat(self.w.args.config_dir).st_mode), 0o700)

    def test_good_config_not_rewritten(self):
        good = "[reticulum]\n  share_instance = Yes\n  instance_name = lan\n  rpc_key = %s\n\n[interfaces]\n" % ("cd" * 32)
        with open(self.path, "w") as f:
            f.write(good)
        os.chmod(self.path, 0o644)
        before = os.stat(self.path).st_mtime_ns
        self.assertEqual(sc.ensure_client_config(self.w.args.config_dir, self.w.log), "lan")
        self.assertEqual(self.read(), good)
        self.assertEqual(os.stat(self.path).st_mtime_ns, before)
        self.assertEqual(stat.S_IMODE(os.stat(self.path).st_mode), 0o600)  # mode tightened

    def test_missing_keys_added_instance_default(self):
        with open(self.path, "w") as f:
            f.write("[reticulum]\n  enable_transport = No\n")
        self.assertEqual(sc.ensure_client_config(self.w.args.config_dir, self.w.log), "111")
        text = self.read()
        self.assertIn("share_instance = Yes", text)
        self.assertIn("instance_name = 111", text)
        self.assertNotIn("rpc_key", text)  # the helper never makes one

    def test_missing_config_written_without_rpc_key(self):
        os.unlink(self.path)
        self.assertEqual(sc.ensure_client_config(self.w.args.config_dir, self.w.log), "111")
        text = self.read()
        self.assertIn("share_instance = Yes", text)
        self.assertNotIn("rpc_key", text)
        self.assertEqual(stat.S_IMODE(os.stat(self.path).st_mode), 0o600)
        self.assertIn("without an rpc_key", self.w.stderr.getvalue())

    def test_interface_declaration_refused(self):
        with open(self.path, "w") as f:
            f.write("[reticulum]\n  share_instance = Yes\n[interfaces]\n  [[Default]]\n    type = AutoInterface\n")
        with self.assertRaises(sc.StartupError) as cm:
            sc.ensure_client_config(self.w.args.config_dir, self.w.log)
        self.assertEqual(cm.exception.code, sc.EXIT_USAGE)

    def test_commented_interface_example_is_fine(self):
        with open(self.path, "w") as f:
            f.write("[reticulum]\n  share_instance = Yes\n# [[Default Interface]]\n")
        sc.ensure_client_config(self.w.args.config_dir, self.w.log)

    def test_instance_name_reported_in_ready(self):
        with open(self.path, "w") as f:
            f.write("[reticulum]\n  share_instance = Yes\n  instance_name = mesh7\n")
        self.w.helper.attach()
        self.assertEqual(self.w.helper.ready_line()["instance"], "mesh7")


class Sending(unittest.TestCase):
    def setUp(self):
        self.w = World()
        self.addCleanup(self.w.close)
        self.peer = self.w.fakes.peer()

    def test_send_hands_one_message_to_router(self):
        self.w.send(self.peer, "m1", "hello é", fields={"8": "abc"})
        (m,) = self.w.router.outbound
        self.assertEqual(m.content, "hello é".encode("utf-8"))
        self.assertEqual(m.destination_hash, self.peer.dest_hash())
        self.assertEqual(m.source_hash, self.w.router.dest.hash)
        self.assertEqual(m.fields, {8: "abc"})
        self.assertEqual(m.desired_method, lib(self.w).OPPORTUNISTIC)  # the library falls back to a link when too big
        self.assertIsNotNone(m.delivered_cb)
        self.assertIsNotNone(m.failed_cb)
        (s,) = self.w.states("m1")
        self.assertEqual((s["state"], s.get("reason")), ("queued", None))
        self.assertEqual(s["hash"], m.hash.hex())

    def test_send_never_auto_resends(self):
        self.w.send(self.peer)
        m = self.w.router.outbound[0]
        m.lib_set(m.SENT)
        m.lib_fail()
        for _ in range(5):
            self.w.clock.advance(60)
            self.w.helper.tick()
        self.assertEqual(len(self.w.router.outbound), 1)

    def test_relay_marker_placement(self):
        marker = {"v": 1, "net": "meshtastic", "id": "!a1b2c3d4", "mid": "deadbeef"}
        self.w.send(self.peer, "r1", "[Hilltop/MT] hi", fields={"relay": marker, "251": "spoof", "5": "x"})
        (m,) = self.w.router.outbound
        self.assertEqual(m.fields[0xFB], "111.relay")
        self.assertEqual(m.fields[0xFC], marker)
        self.assertEqual(m.fields[5], "x")
        self.assertEqual(set(m.fields), {5, 0xFB, 0xFC})

    def test_marker_must_be_well_formed(self):
        for bad in ({"v": 1, "net": "m", "id": "i"}, {"v": "1", "net": "m", "id": "i", "mid": "m"},
                    {"v": 1, "net": "", "id": "i", "mid": "m"}, {"v": 1, "net": "m", "id": "i" * 300, "mid": "m"},
                    {"v": 999, "net": "m", "id": "i", "mid": "m"}, "x", None):
            self.w.send(self.peer, "bad", "t", fields={"relay": bad})
        self.assertEqual(self.w.router.outbound, [])
        self.assertEqual(self.w.helper.malformed, 7)

    def test_fields_passthrough_json_values_and_int_keys(self):
        f = sc.fields_from_json({"8": [1, "a", {"k": None}], "1": 2.5, "9": True})
        self.assertEqual(f, {8: [1, "a", {"k": None}], 1: 2.5, 9: True})
        for bad in ({"256": 1}, {"-1": 1}, {"x": 1}, {"1": float("inf")}):
            with self.assertRaises(sc.BadCommand):
                sc.fields_from_json(bad)
        deep = 1
        for _ in range(20):
            deep = [deep]
        with self.assertRaises(sc.BadCommand):
            sc.fields_from_json({"1": deep})

    def test_peer_unannounced_parks_and_resumes_on_heard(self):
        stranger = self.w.fakes.peer(announced=False)
        self.w.send(stranger, "p1", "wait for me")
        self.assertEqual(self.w.router.outbound, [])
        (s,) = self.w.states("p1")
        self.assertEqual((s["state"], s["reason"]), ("queued", "peer_unannounced"))
        self.assertNotIn("hash", s)
        # another peer's announce does not release it
        self.w.fakes.announce(self.w.fakes.peer(announced=False), "Other")
        self.assertEqual(self.w.router.outbound, [])
        # the matching announce does: heard, then the send is handed over, once
        d = self.w.fakes.announce(stranger, "Bob")
        heard = [h for h in self.w.of("heard") if h["dest"] == d.hex()]
        self.assertEqual(heard, [{"type": "heard", "dest": d.hex(), "name": "Bob"}])
        (m,) = self.w.router.outbound
        self.assertEqual(m.content, b"wait for me")
        self.w.fakes.announce(stranger, "Bob")  # a second announce must not resend
        self.assertEqual(len(self.w.router.outbound), 1)
        self.assertEqual([x["state"] for x in self.w.states("p1")], ["queued"])  # the parked row stays the only queued
        m.lib_deliver()
        self.assertEqual(self.w.states("p1")[-1]["state"], "delivered")
        self.assertEqual(self.w.states("p1")[-1]["hash"], m.hash.hex())

    def test_parked_cap(self):
        for i in range(sc.MAX_PARKED):
            self.w.send(self.w.fakes.peer(announced=False), "k%d" % i)
        self.w.send(self.w.fakes.peer(announced=False), "overflow")
        last = self.w.states("overflow")[-1]
        self.assertEqual((last["state"], last["reason"]), ("failed", "helper_busy"))

    def test_send_table_bounded(self):
        for i in range(sc.MAX_SENDS):
            self.w.send(self.peer, "s%d" % i)
        self.w.send(self.peer, "over")
        self.assertEqual(self.w.states("over")[-1]["reason"], "helper_busy")
        # a concluded send frees a slot
        first = self.w.router.outbound[0]
        first.lib_deliver()
        self.w.send(self.peer, "again")
        self.assertEqual(self.w.states("again")[-1]["state"], "queued")

    def test_router_error_is_a_failed_state_not_a_crash(self):
        def boom(m):
            raise IOError(FOREIGN)

        self.w.router.handle_outbound = boom
        self.w.send(self.peer, "e1")
        s = self.w.states("e1")[-1]
        self.assertEqual((s["state"], s["reason"]), ("failed", "send_error"))
        self.assertNotIn(FOREIGN, self.w.stderr.getvalue())

    def test_router_is_called_without_the_state_lock(self):
        """handle_outbound waits for the router's outbound lock, whose holder may be calling our failed
        callback; holding our lock across it would deadlock (found by reading lxmf 1.2.1)."""
        free = []
        orig = self.w.router.handle_outbound

        def probe(m):
            done = threading.Event()

            def other():
                with self.w.helper.lock:
                    done.set()

            th = threading.Thread(target=other, daemon=True)
            th.start()
            free.append(done.wait(3))
            th.join(1)
            orig(m)

        self.w.router.handle_outbound = probe
        self.w.send(self.peer, "dl1")
        stranger = self.w.fakes.peer(announced=False)
        self.w.send(stranger, "dl2")
        self.w.fakes.announce(stranger, "S")  # the parked send resumes through the same path
        self.assertEqual(free, [True, True])

    def test_duplicate_id_in_flight_ignored(self):
        self.w.send(self.peer, "d1")
        self.w.send(self.peer, "d1", "again")
        self.assertEqual(len(self.w.router.outbound), 1)

    def test_destination_mismatch_fails_closed(self):
        liar = self.w.fakes.peer(announced=False)
        other = self.w.fakes.peer(announced=False)
        self.w.fakes.known[liar.dest_hash()] = other  # recall returns the wrong identity
        self.w.send(liar, "x1")
        self.assertEqual(self.w.router.outbound, [])
        self.assertEqual(self.w.states("x1")[-1]["state"], "failed")


class StateMachine(unittest.TestCase):
    """The v0.3 mapping, the monotone rule and the seven-library-state matrix."""

    def setUp(self):
        self.w = World()
        self.addCleanup(self.w.close)
        self.peer = self.w.fakes.peer()
        self.w.send(self.peer, "m")
        self.m = self.w.router.outbound[0]

    def seq(self, sid="m"):
        return [(s["state"], s.get("reason")) for s in self.w.states(sid)]

    def test_normal_path_queued_sent_delivered(self):
        self.m.lib_set(self.m.SENDING)
        self.w.clock.advance(1.5)
        self.w.helper.tick()
        self.m.lib_set(self.m.SENT)
        self.w.clock.advance(1.5)
        self.w.helper.tick()
        self.m.lib_deliver()
        self.assertEqual(self.seq(), [("queued", None), ("sent", None), ("delivered", None)])

    def test_late_lower_state_after_delivered_ignored(self):
        self.m.lib_deliver()
        n = len(self.w.stdout.objs())
        self.m.lib_set(self.m.OUTBOUND)
        self.w.helper._observe(self.w.helper.sends["m"], self.m)
        self.m.lib_fail()  # the R07 check 7 stale 'failed' after 'delivered'
        self.m.lib_set(self.m.SENT)
        self.w.helper._observe(self.w.helper.sends["m"], self.m)
        self.assertEqual(len(self.w.stdout.objs()), n)
        self.assertEqual(self.seq()[-1], ("delivered", None))

    def test_failed_after_air_keeps_sent_transmitted_not_confirmed(self):
        self.m.lib_set(self.m.SENT)
        self.w.clock.advance(1.5)
        self.w.helper.tick()
        self.m.lib_fail()
        self.assertEqual(self.seq(), [("queued", None), ("sent", None), ("sent", "transmitted, not confirmed")])

    def test_failed_after_air_seen_only_at_failure(self):
        # air was never sampled (no tick between): FAILED alone is the pre-air branch
        self.m.lib_fail()
        self.assertEqual(self.seq()[-1], ("queued", "peer_unannounced"))

    def test_failed_while_outbound_is_queued_peer_unannounced(self):
        self.m.lib_fail()
        self.assertEqual(self.seq(), [("queued", None), ("queued", "peer_unannounced")])

    def test_rejected_and_cancelled_are_failed(self):
        self.m.lib_fail(self.m.REJECTED)
        self.assertEqual(self.seq()[-1], ("failed", "rejected"))
        self.w.send(self.peer, "c")
        c = self.w.router.outbound[1]
        c.lib_set(c.SENDING)
        c.lib_fail(c.CANCELLED)
        self.assertEqual(self.seq("c")[-1], ("failed", "cancelled"))

    def test_propagated_sent_is_relayed(self):
        self.m.method = self.m.PROPAGATED
        self.m.lib_propagated()
        self.assertEqual(self.seq()[-1], ("relayed", None))
        self.m.lib_fail()  # nothing after relayed
        self.assertEqual(self.seq()[-1], ("relayed", None))

    def test_resend_after_post_air_failure_is_a_new_attempt_same_id_monotone(self):
        self.m.lib_set(self.m.SENT)
        self.w.clock.advance(1.5)
        self.w.helper.tick()
        self.m.lib_fail()
        self.w.send(self.peer, "m", "retry")  # 111d re-queues on heard
        self.assertEqual(len(self.w.router.outbound), 2)
        m2 = self.w.router.outbound[1]
        self.assertEqual(m2.content, b"retry")
        before = self.seq()
        m2.lib_set(m2.SENT)
        self.w.clock.advance(1.5)
        self.w.helper.tick()
        self.assertEqual(self.seq(), before)  # sent again: nothing new, never a regression to queued
        m2.lib_deliver()
        self.assertEqual(self.seq()[-1], ("delivered", None))

    def test_stale_attempt_delivery_is_honoured_other_stale_states_are_not(self):
        self.m.lib_set(self.m.SENT)
        self.w.clock.advance(1.5)
        self.w.helper.tick()
        self.m.lib_fail()
        self.w.send(self.peer, "m", "retry")
        m2 = self.w.router.outbound[1]
        n = len(self.w.stdout.objs())
        self.m.lib_fail(self.m.REJECTED)  # an old attempt reports something: ignored
        self.assertEqual(len(self.w.stdout.objs()), n)
        self.m.lib_deliver()  # the old attempt was in fact acknowledged late
        self.assertEqual(self.seq()[-1], ("delivered", None))
        self.assertIsNotNone(m2)

    def test_final_ids_cannot_be_resent(self):
        self.m.lib_deliver()
        self.w.send(self.peer, "m", "again")
        self.assertEqual(len(self.w.router.outbound), 1)

    def test_sampling_runs_only_while_something_is_in_flight(self):
        h = self.w.helper
        self.assertTrue(h._pending())
        self.m.lib_deliver()
        self.assertFalse(h._pending())
        self.assertEqual(h.next_wakeup(), max(0.0, h.next_link_poll - self.w.clock()))
        self.w.send(self.peer, "n")
        self.assertLessEqual(h.next_wakeup(), sc.STATE_TICK_SECS)

    # ---- the matrix: every library state x every prior emitted state
    CASES = {  # name: (library state attr, propagated, air) -> mapped
        "OUTBOUND": ("OUTBOUND", False, False),
        "SENDING": ("SENDING", False, True),
        "SENT": ("SENT", False, True),
        "SENT-propagated": ("SENT", True, True),
        "DELIVERED": ("DELIVERED", False, True),
        "FAILED-after-air": ("FAILED", False, True),
        "FAILED-before-air": ("FAILED", False, False),
        "REJECTED": ("REJECTED", False, False),
        "CANCELLED": ("CANCELLED", False, False),
    }
    MAPPED = {
        "OUTBOUND": ("queued", None), "SENDING": ("sent", None), "SENT": ("sent", None),
        "SENT-propagated": ("relayed", None), "DELIVERED": ("delivered", None),
        "FAILED-after-air": ("sent", "transmitted, not confirmed"),
        "FAILED-before-air": ("queued", "peer_unannounced"),
        "REJECTED": ("failed", "rejected"), "CANCELLED": ("failed", "cancelled"),
    }
    # emitted? per prior state, in CASES order
    EXPECT = {
        None: dict.fromkeys(CASES, True),
        ("queued", None): {"OUTBOUND": False, "SENDING": True, "SENT": True, "SENT-propagated": True, "DELIVERED": True,
                           "FAILED-after-air": True, "FAILED-before-air": True, "REJECTED": True, "CANCELLED": True},
        ("queued", "peer_unannounced"): {"OUTBOUND": False, "SENDING": True, "SENT": True, "SENT-propagated": True,
                                         "DELIVERED": True, "FAILED-after-air": True, "FAILED-before-air": False,
                                         "REJECTED": True, "CANCELLED": True},
        ("sent", None): {"OUTBOUND": False, "SENDING": False, "SENT": False, "SENT-propagated": True, "DELIVERED": True,
                         "FAILED-after-air": True, "FAILED-before-air": False, "REJECTED": True, "CANCELLED": True},
        ("sent", "transmitted, not confirmed"): {"OUTBOUND": False, "SENDING": False, "SENT": False,
                                                 "SENT-propagated": True, "DELIVERED": True, "FAILED-after-air": False,
                                                 "FAILED-before-air": False, "REJECTED": True, "CANCELLED": True},
        ("relayed", None): {"OUTBOUND": False, "SENDING": False, "SENT": False, "SENT-propagated": False,
                            "DELIVERED": True, "FAILED-after-air": False, "FAILED-before-air": False,
                            "REJECTED": False, "CANCELLED": False},
        ("delivered", None): dict.fromkeys(CASES, False),
        ("failed", "rejected"): dict.fromkeys(CASES, False),
    }

    def test_matrix_library_state_by_prior_state(self):
        c = lib(self.w)
        for prior, row in self.EXPECT.items():
            for name, (attr, prop, air) in self.CASES.items():
                mapped = sc.map_library_state(getattr(c, attr), prop, air, c)
                self.assertEqual(mapped, self.MAPPED[name], name)
                pstate, preason = prior if prior else (None, None)
                self.assertEqual(sc.may_emit(pstate, preason, *mapped), row[name], "%s after %s" % (name, prior))

    def test_unknown_library_state_maps_to_nothing(self):
        self.assertIsNone(sc.map_library_state(0x77, False, False, lib(self.w)))

    def test_every_emitted_sequence_is_monotone(self):
        # drive every ordered pair of library states through the live path and check the rank never drops
        rank = {"queued": 1, "sent": 2, "relayed": 3, "delivered": 4, "failed": 5}
        names = ["OUTBOUND", "SENDING", "SENT", "DELIVERED", "FAILED", "REJECTED", "CANCELLED"]
        n = 0
        for a in names:
            for b in names:
                n += 1
                sid = "z%d" % n
                self.w.send(self.peer, sid)
                m = self.w.router.outbound[-1]
                e = self.w.helper.sends[sid]
                for nm in (a, b):
                    m.state = getattr(m, nm)
                    self.w.helper._observe(e, m)
                ranks = [rank[s["state"]] for s in self.w.states(sid)]
                self.assertEqual(ranks, sorted(ranks), (a, b))
                finals = [s for s in self.w.states(sid) if s["state"] in ("delivered", "failed")]
                self.assertLessEqual(len(finals), 1, (a, b))


class Receiving(unittest.TestCase):
    def setUp(self):
        self.w = World()
        self.addCleanup(self.w.close)

    def last_msg(self):
        return self.w.of("msg")[-1]

    def test_signed_message_shape(self):
        peer = self.w.fakes.peer()
        self.w.fakes.announce(peer, "Alice")
        m = inbound(self.w, peer, "hello é".encode("utf-8"), signed=True)
        self.w.router.delivery_cb(m)
        o = self.last_msg()
        self.assertEqual(set(o), {"type", "hash", "src", "name", "text", "claim_ts", "signed", "why_unsigned", "fields"})
        self.assertEqual(o["hash"], m.hash.hex())
        self.assertEqual(o["src"], peer.dest_hash().hex())
        self.assertEqual(o["name"], "Alice")
        self.assertEqual(o["text"], "hello é")
        self.assertEqual(o["claim_ts"], 1700000000.5)
        self.assertIs(o["signed"], True)
        self.assertIsNone(o["why_unsigned"])
        self.assertEqual(o["fields"], {})

    def test_unsigned_reasons(self):
        for reason, why in ((sc.LibStates(t.FakeMessage).GENERATING + 1, "source_unknown"), (2, "signature_invalid"), (None, "unverified"), (9, "unverified")):
            peer = self.w.fakes.peer()
            self.w.router.delivery_cb(inbound(self.w, peer, b"x", signed=False, reason=reason))
            o = self.last_msg()
            self.assertIs(o["signed"], False)
            self.assertEqual(o["why_unsigned"], why)

    def test_name_unknown_is_null_and_recalled_from_app_data(self):
        peer = self.w.fakes.peer()
        self.w.router.delivery_cb(inbound(self.w, peer, b"x"))
        self.assertIsNone(self.last_msg()["name"])
        peer2 = self.w.fakes.peer()
        self.w.fakes.app_data[peer2.dest_hash()] = b"Carol"  # known from an earlier session
        self.w.router.delivery_cb(inbound(self.w, peer2, b"x"))
        self.assertEqual(self.last_msg()["name"], "Carol")

    def test_every_delivery_emitted_once_dedupe_is_111d(self):
        peer = self.w.fakes.peer()
        m = inbound(self.w, peer, b"x")
        self.w.router.delivery_cb(m)
        self.assertEqual(len(self.w.of("msg")), 1)
        self.w.router.delivery_cb(inbound(self.w, peer, b"y"))
        self.assertEqual(len(self.w.of("msg")), 2)
        self.assertNotEqual(self.w.of("msg")[0]["hash"], self.w.of("msg")[1]["hash"])

    def test_text_capped_and_marked(self):
        text = ("a" * (sc.TEXT_CAP + 5000)).encode()
        self.w.router.delivery_cb(inbound(self.w, None, text))
        o = self.last_msg()
        self.assertIs(o["truncated"], True)
        self.assertLessEqual(len(o["text"].encode("utf-8")), sc.TEXT_CAP)
        self.assertLessEqual(len(self.w.stdout.lines()[-1].encode("utf-8")), sc.MAX_LINE)

    def test_cap_cuts_on_a_code_point(self):
        text = ("é" * (sc.TEXT_CAP // 2 + 10)).encode("utf-8")
        self.w.router.delivery_cb(inbound(self.w, None, text))
        o = self.last_msg()
        self.assertNotIn("�", o["text"])

    def test_line_limit_holds_for_worst_case_escaping(self):
        text = ('"\n\\' * (sc.TEXT_CAP // 3)).encode()  # every char escapes to two
        self.w.router.delivery_cb(inbound(self.w, None, text))
        line = self.w.stdout.lines()[-1]
        self.assertLessEqual(len(line.encode("utf-8")), sc.MAX_LINE)
        self.assertIs(self.last_msg()["truncated"], True)
        ctrl = "\x01".encode() * sc.TEXT_CAP  # a control char escapes to six
        self.w.router.delivery_cb(inbound(self.w, None, ctrl))
        self.assertLessEqual(len(self.w.stdout.lines()[-1].encode("utf-8")), sc.MAX_LINE)

    def test_invalid_utf8_and_missing_hash(self):
        self.w.router.delivery_cb(inbound(self.w, None, b"ok \xff\xfe bad"))
        self.assertIn("�", self.last_msg()["text"])
        n = len(self.w.of("msg"))
        m = inbound(self.w, None, b"x")
        m.hash = None
        self.w.router.delivery_cb(m)
        self.assertEqual(len(self.w.of("msg")), n)

    def test_attachments_are_not_forwarded(self):
        secret = b"\xde\xad\xbe\xef" * 100
        fields = {5: [["report.pdf", secret], ["b.bin", b"12345"]], 6: ["jpg", secret], 7: [0x10, b"\x00" * 33],
                  8: b"thread-id-bytes"}
        self.w.router.delivery_cb(inbound(self.w, None, b"see attached", fields=fields))
        raw = self.w.stdout.lines()[-1]
        o = self.last_msg()
        self.assertEqual(o["fields"]["5"], {"attachment": {"bytes": 405}})
        self.assertEqual(o["fields"]["6"], {"attachment": {"bytes": 400}})
        self.assertEqual(o["fields"]["7"], {"attachment": {"bytes": 33}})
        self.assertNotIn("deadbeef", raw)
        self.assertNotIn("report.pdf", raw)
        self.assertEqual(o["fields"]["8"], b"thread-id-bytes".hex())  # other bytes: hex

    def test_bytes_fields_hex_and_capped(self):
        self.w.router.delivery_cb(inbound(self.w, None, b"x", fields={9: b"\x01" * 1000}))
        o = self.last_msg()
        self.assertEqual(len(o["fields"]["9"]), sc.FIELD_BYTES_CAP * 2)
        self.assertIs(o["fields_truncated"], True)

    def test_ticket_dropped_and_odd_keys_dropped(self):
        self.w.router.delivery_cb(inbound(self.w, None, b"x", fields={0x0C: b"T" * 16, "str-key": 1, 8: 1}))
        o = self.last_msg()
        self.assertEqual(o["fields"], {"8": 1})
        self.assertIs(o["fields_truncated"], True)

    def test_relay_marker_round_trip(self):
        marker = {"v": 1, "net": "meshtastic", "id": "!a1b2c3d4", "mid": "deadbeef"}
        self.w.send(self.w.fakes.peer(), "rt", "[x/MT] hi", fields={"relay": marker})
        sent = self.w.router.outbound[0]
        # what the far helper's router delivers: the very fields dict, msgpack round-tripped
        self.w.router.delivery_cb(inbound(self.w, None, sent.content, fields=dict(sent.fields)))
        o = self.last_msg()
        self.assertEqual(o["fields"]["relay"], marker)
        self.assertEqual(o["fields"]["251"], "111.relay")
        self.assertEqual(o["fields"]["252"], marker)

    def test_relay_marker_bytes_strings_and_malformed(self):
        wire = {0xFB: b"111.relay", 0xFC: {b"v": 1, b"net": b"meshtastic", b"id": b"!a", b"mid": b"m"}}
        self.w.router.delivery_cb(inbound(self.w, None, b"x", fields=wire))
        self.assertEqual(self.last_msg()["fields"]["relay"], {"v": 1, "net": "meshtastic", "id": "!a", "mid": "m"})
        for bad in ({0xFB: "111.relay", 0xFC: {"v": 1}}, {0xFB: "other", 0xFC: {"v": 1, "net": "n", "id": "i", "mid": "m"}},
                    {0xFB: "111.relay", 0xFC: "str"}, {0xFC: {"v": 1, "net": "n", "id": "i", "mid": "m"}}):
            self.w.router.delivery_cb(inbound(self.w, None, b"x", fields=bad))
            self.assertNotIn("relay", self.last_msg()["fields"])

    def test_foreign_field_cannot_inject_relay_key(self):
        self.w.router.delivery_cb(inbound(self.w, None, b"x", fields={"relay": {"v": 1, "net": "n", "id": "i", "mid": "m"}}))
        self.assertNotIn("relay", self.last_msg()["fields"])

    def test_deep_and_wide_fields_bounded(self):
        deep = 1
        for _ in range(50):
            deep = {1: deep}
        wide = {i: i for i in range(500)}
        self.w.router.delivery_cb(inbound(self.w, None, b"x", fields={10: deep, 11: wide, 12: "s" * 5000}))
        o = self.last_msg()
        self.assertIs(o["fields_truncated"], True)
        self.assertLessEqual(len(json.dumps(o["fields"])), sc.FIELDS_JSON_CAP)
        self.assertLessEqual(len(o["fields"]["11"]), sc.FIELD_ITEMS_CAP)

    def test_claim_ts_odd_values(self):
        for ts, want in ((None, None), (float("nan"), None), (float("inf"), None), ("x", None), (5, 5.0), (True, None)):
            self.w.router.delivery_cb(inbound(self.w, None, b"x" + repr(ts).encode(), ts=ts))
            self.assertEqual(self.last_msg()["claim_ts"], want, ts)

    def test_heard_for_every_announce_with_name(self):
        p = self.w.fakes.peer(announced=False)
        d = self.w.fakes.announce(p, "Dave\x07é")
        self.assertEqual(self.w.of("heard")[-1], {"type": "heard", "dest": d.hex(), "name": "Daveé"})
        d2 = self.w.fakes.announce(self.w.fakes.peer(announced=False), None)
        self.assertEqual(self.w.of("heard")[-1], {"type": "heard", "dest": d2.hex(), "name": None})

    def test_own_announce_not_reported(self):
        for h in self.w.fakes.transport.handlers:
            h.received_announce(bytes(self.w.router.dest.hash), self.w.helper.identity, b"me")
        self.assertEqual(self.w.of("heard"), [])

    def test_announce_handler_registered_for_lxmf_delivery(self):
        (h,) = self.w.fakes.transport.handlers
        self.assertEqual(h.aspect_filter, "lxmf.delivery")
        import inspect
        self.assertEqual(len(inspect.signature(h.received_announce).parameters), 3)

    def test_bad_app_data_is_not_a_crash(self):
        p = self.w.fakes.peer(announced=False)
        d = p.dest_hash()
        for h in self.w.fakes.transport.handlers:
            h.received_announce(d, p, b"\xff\xfe")  # the fake decoder raises
        self.assertEqual(self.w.of("heard")[-1]["name"], None)


class Timers(unittest.TestCase):
    def test_announce_on_start_then_every_interval_on_the_monotonic_clock(self):
        w = World(announce_interval=60)
        self.addCleanup(w.close)
        self.assertEqual(len(w.router.announced), 1)
        w.clock.advance(59 * 60)
        w.helper.tick()
        self.assertEqual(len(w.router.announced), 1)
        w.clock.advance(60)
        w.helper.tick()
        self.assertEqual(len(w.router.announced), 2)
        w.clock.advance(3600)
        w.helper.tick()
        self.assertEqual(len(w.router.announced), 3)

    def test_interval_zero_is_events_only(self):
        w = World(announce_interval=0)
        self.addCleanup(w.close)
        self.assertEqual(len(w.router.announced), 1)  # the start announce
        w.clock.advance(10 * 24 * 3600)
        w.helper.tick()
        self.assertEqual(len(w.router.announced), 1)
        self.assertIsNone(w.helper.next_announce)
        w.cmd({"type": "announce"})
        self.assertEqual(len(w.router.announced), 2)

    def test_command_announce_resets_the_timer(self):
        w = World(announce_interval=60)
        self.addCleanup(w.close)
        w.clock.advance(50 * 60)
        w.cmd({"type": "announce"})
        w.clock.advance(50 * 60)
        w.helper.tick()
        self.assertEqual(len(w.router.announced), 2)  # not at +60 from start
        w.clock.advance(10 * 60)
        w.helper.tick()
        self.assertEqual(len(w.router.announced), 3)

    def test_link_down_up_reports_and_announces_on_up(self):
        w = World(announce_interval=0)
        self.addCleanup(w.close)
        iface = w.fakes.transport.interfaces[0]
        n = len(w.router.announced)
        iface.online = False
        w.clock.advance(sc.LINK_POLL_SECS + 0.1)
        w.helper.tick()
        self.assertEqual(w.of("link")[-1]["state"], "down")
        self.assertEqual(len(w.router.announced), n)  # a down announces nothing
        w.clock.advance(sc.LINK_POLL_SECS + 0.1)
        w.helper.tick()
        self.assertEqual([l["state"] for l in w.of("link")], ["up", "down"])  # no repeat while unchanged
        iface.online = True
        w.clock.advance(sc.LINK_POLL_SECS + 0.1)
        w.helper.tick()
        self.assertEqual([l["state"] for l in w.of("link")], ["up", "down", "up"])
        self.assertEqual(len(w.router.announced), n + 1)

    def test_link_poll_not_faster_than_its_period(self):
        w = World()
        self.addCleanup(w.close)
        iface = w.fakes.transport.interfaces[0]
        iface.online = False
        w.clock.advance(1)
        w.helper.tick()
        self.assertEqual([l["state"] for l in w.of("link")], ["up"])

    def test_missing_client_interface_reads_as_down(self):
        w = World()
        self.addCleanup(w.close)
        w.fakes.transport.interfaces[:] = []
        w.clock.advance(sc.LINK_POLL_SECS + 0.1)
        w.helper.tick()
        self.assertEqual(w.of("link")[-1]["state"], "down")

    def test_idle_helper_wakes_only_for_link_and_announce(self):
        w = World(announce_interval=60)
        self.addCleanup(w.close)
        self.assertAlmostEqual(w.helper.next_wakeup(), sc.LINK_POLL_SECS)


class MainEntry(unittest.TestCase):
    """main(): exit codes and the whole life cycle over injected streams."""

    def setUp(self):
        self.fakes = None
        self.tmp = t.tempfile.TemporaryDirectory(prefix="lxmf-sidecar-main-")
        self.addCleanup(self.tmp.cleanup)
        self.args = t.Args(self.tmp.name)

    def run_main(self, fakes, stdin=b"", argv=None):
        self.fakes = fakes
        if fakes is not None:
            fakes.install()
            self.addCleanup(fakes.uninstall)
        out, err = t.Rec(), io.StringIO()
        rc = sc.main(argv or self.args.argv(), stdin=io.BytesIO(stdin), stdout=out, stderr=err, clock=t.FakeClock())
        return rc, out, err

    def test_clean_run_ready_first_then_quit(self):
        rc, out, err = self.run_main(t.Fakes("client"), b'{"type":"announce"}\n{"type":"quit"}\n')
        self.assertEqual(rc, 0)
        objs = out.objs()
        self.assertEqual(objs[0]["type"], "ready")
        self.assertEqual([o["type"] for o in objs], ["ready", "link"])
        self.assertTrue(self.fakes.exit_handler_ran)

    def test_stdin_eof_is_a_clean_exit(self):
        rc, out, err = self.run_main(t.Fakes("client"), b"")
        self.assertEqual(rc, 0)

    def test_malformed_lines_do_not_stop_it(self):
        rc, out, err = self.run_main(t.Fakes("client"), b"junk\n" + b"x" * (sc.MAX_LINE + 10) + b'\n{"type":"quit"}\n')
        self.assertEqual(rc, 0)
        self.assertEqual(err.getvalue().count("ignored a malformed command"), 2)

    def test_became_the_instance_is_partitioned_exit_3(self):
        rc, out, err = self.run_main(t.Fakes("instance"))
        self.assertEqual(rc, 3)
        self.assertEqual(out.objs(), [{"type": "link", "state": "partitioned"}])
        self.assertTrue(self.fakes.reticulum_calls[0]["require_shared_instance"])

    def test_rns_without_require_mode_relies_on_the_connected_flag(self):
        rc, out, err = self.run_main(t.Fakes("old_instance"))
        self.assertEqual(rc, 3)
        self.assertEqual(out.objs(), [{"type": "link", "state": "partitioned"}])
        self.assertNotIn("require_shared_instance", self.fakes.reticulum_calls[0])

    def test_missing_libraries_exit_4_naming_the_venv(self):
        import sys
        saved = {k: sys.modules.get(k) for k in ("RNS", "LXMF")}
        sys.modules["RNS"] = None  # makes `import RNS` raise ImportError
        sys.modules["LXMF"] = None
        try:
            rc, out, err = self.run_main(None)
        finally:
            for k, v in saved.items():
                if v is None:
                    sys.modules.pop(k, None)
                else:
                    sys.modules[k] = v
        self.assertEqual(rc, 4)
        self.assertEqual(out.getvalue(), b"")
        self.assertIn("venvs/lxmf", err.getvalue())

    def test_usage_errors_exit_2(self):
        rc, out, err = self.run_main(t.Fakes("client"), argv=["--identity", "x"])
        self.assertEqual(rc, 2)
        rc, out, err = self.run_main(t.Fakes("client"), argv=self.args.argv() + ["--announce-interval", "-1"])
        self.assertEqual(rc, 2)
        rc, out, err = self.run_main(t.Fakes("client"), argv=self.args.argv() + ["--announce-interval", "x"])
        self.assertEqual(rc, 2)
        self.assertEqual(out.getvalue(), b"")

    def test_help_exits_0_without_the_libraries(self):
        import contextlib
        with contextlib.redirect_stdout(io.StringIO()) as shown:
            rc, out, err = self.run_main(None, argv=["--help"])
        self.assertEqual(rc, 0)
        self.assertIn("--config-dir", shown.getvalue())

    def test_interface_in_client_config_exit_1_or_2_not_a_crash(self):
        os.makedirs(self.args.config_dir, mode=0o700)
        with open(os.path.join(self.args.config_dir, "config"), "w") as f:
            f.write("[reticulum]\n[interfaces]\n [[X]]\n  type = AutoInterface\n")
        rc, out, err = self.run_main(t.Fakes("client"))
        self.assertEqual(rc, 2)
        self.assertEqual(out.getvalue(), b"")

    def test_unreadable_identity_exit_1(self):
        os.makedirs(os.path.dirname(self.args.identity), exist_ok=True)
        with open(self.args.identity, "wb") as f:
            f.write(b"junk")
        rc, out, err = self.run_main(t.Fakes("client"))
        self.assertEqual(rc, 1)

    def test_no_foreign_text_and_no_key_material_on_stderr(self):
        fk = t.Fakes("client")
        fk.install()
        self.addCleanup(fk.uninstall)
        stranger = fk.peer(announced=False)
        cmds = [
            b'{"type":"send","id":"s1","to":"%s","text":"%s"}\n' % (stranger.dest_hash().hex().encode(), FOREIGN.encode()),
            ('{"type":"bogus","x":"%s"}\n' % FOREIGN).encode(),
            ('not json %s\n' % FOREIGN).encode(),
            ('{"type":"name","display":"%s"}\n' % FOREIGN).encode(),
        ]
        out, err = t.Rec(), io.StringIO()
        peer = fk.peer()

        # run the real loop, but inject traffic from the "library side" through a thread-safe hook
        class Stdin(io.BytesIO):
            pass

        stdin = Stdin(b"".join(cmds) + b'{"type":"quit"}\n')
        orig_attach = sc.Helper.attach

        def attach_and_inject(self_):
            orig_attach(self_)
            fk.announce(stranger, FOREIGN)
            self_.router.delivery_cb(inbound(type("W", (), {"helper": self_, "fakes": fk})(), peer,
                                             ("body " + FOREIGN).encode(), fields={8: FOREIGN}, signed=False, reason=1))

        sc.Helper.attach = attach_and_inject
        try:
            rc = sc.main(self.args.argv() + ["-v"], stdin=stdin, stdout=out, stderr=err, clock=t.FakeClock())
        finally:
            sc.Helper.attach = orig_attach
        self.assertEqual(rc, 0)
        self.assertIn(FOREIGN, out.getvalue().decode("utf-8"))  # it did flow on the protocol...
        self.assertNotIn(FOREIGN, err.getvalue())              # ...and never reached the log
        with open(self.args.identity, "rb") as f:
            prv = f.read()
        self.assertNotIn(prv.hex(), err.getvalue())
        self.assertNotIn(prv.hex(), out.getvalue().decode("utf-8"))
        self.assertTrue(all(" " <= c < "\x7f" or c == "\n" for c in err.getvalue()))

    def test_state_lines_flow_through_the_real_loop(self):
        fk = t.Fakes("client")
        peer = fk.peer()
        line = b'{"type":"send","id":"a","to":"%s","text":"hi"}\n' % peer.dest_hash().hex().encode()
        rc, out, err = self.run_main(fk, line + b'{"type":"quit"}\n')
        self.assertEqual(rc, 0)
        self.assertEqual([o["state"] for o in out.objs() if o["type"] == "state"], ["queued"])


class SourceRules(unittest.TestCase):
    """AST scans: the helper listens on no port, uses no wall clock, prints nothing to stdout."""

    @classmethod
    def setUpClass(cls):
        with open(os.path.join(t.ROOT, "lxmf_sidecar.py"), encoding="utf-8") as f:
            cls.src = f.read()
        cls.tree = ast.parse(cls.src)

    def test_no_socket_import_bind_or_listen(self):
        banned_mods = {"socket", "socketserver", "http", "http.server", "urllib", "urllib.request", "asyncio",
                       "ssl", "ftplib", "smtplib", "xmlrpc", "multiprocessing", "subprocess"}
        for node in ast.walk(self.tree):
            if isinstance(node, ast.Import):
                for a in node.names:
                    self.assertNotIn(a.name.split(".")[0] if a.name.split(".")[0] in banned_mods else a.name, banned_mods, a.name)
            elif isinstance(node, ast.ImportFrom):
                self.assertNotIn((node.module or "").split(".")[0], banned_mods)
            elif isinstance(node, ast.Attribute):
                self.assertNotIn(node.attr, ("bind", "listen", "accept", "connect", "create_server", "serve_forever"), node.attr)

    def test_no_socket_text_anywhere(self):
        for word in ("socket.", "AF_INET", "bind(", ".listen("):
            self.assertNotIn(word, self.src)

    def test_no_wall_clock(self):
        for node in ast.walk(self.tree):
            if isinstance(node, ast.Attribute) and isinstance(node.value, ast.Name) and node.value.id == "time":
                self.assertIn(node.attr, ("monotonic",), "time.%s" % node.attr)
            if isinstance(node, ast.ImportFrom) and node.module in ("time", "datetime"):
                self.fail("from %s import ..." % node.module)

    def test_no_print_and_no_direct_stdout(self):
        for node in ast.walk(self.tree):
            if isinstance(node, ast.Call) and isinstance(node.func, ast.Name):
                self.assertNotEqual(node.func.id, "print")
        # sys.stdout is touched only to flush it at exit
        uses = [n for n in ast.walk(self.tree) if isinstance(n, ast.Attribute) and n.attr == "stdout"
                and isinstance(n.value, ast.Name) and n.value.id == "sys"]
        self.assertLessEqual(len(uses), 1)

    def test_libraries_imported_lazily_inside_main(self):
        for node in self.tree.body:
            if isinstance(node, (ast.Import, ast.ImportFrom)):
                names = [a.name for a in node.names] + [getattr(node, "module", None) or ""]
                self.assertFalse(any(n.split(".")[0] in ("RNS", "LXMF") for n in names))

    def test_python37_syntax_floor(self):
        # no walrus, no match, no PEP 604 at runtime (annotations are strings via __future__)
        for node in ast.walk(self.tree):
            self.assertNotIsInstance(node, ast.NamedExpr)
        self.assertIn("from __future__ import annotations", self.src)

    def test_no_model_names_in_the_shipped_files(self):
        words = ["cla" + "ude", "anthr" + "opic", "son" + "net", "op" + "us", "hai" + "ku", "fa" + "ble", "ki" + "mi"]
        for name in ("lxmf_sidecar.py", "gen_lock.py", "README.md", "LICENSE-NOTE.md", "requirements.in"):
            with open(os.path.join(t.ROOT, name), encoding="utf-8") as f:
                low = f.read().lower()
            for w in words:
                self.assertNotIn(w, low, "%s mentions %s" % (name, w))


class ProcessLevel(unittest.TestCase):
    """The real __main__ path in a subprocess: fd 1 stays the protocol, library noise goes to stderr,
    SIGTERM is a clean exit. Uses the fake surface through a tiny runner script."""

    RUNNER = r'''
import os, sys, runpy
sys.path.insert(0, %(tests)r)
import _support as t
fk = t.Fakes(%(mode)r).install()
class Noisy(fk.rns.Reticulum):
    def __init__(self, *a, **k):
        print("LIBRARY NOISE ON STDOUT")
        os.write(1, b"RAW NOISE ON FD1\n")
        super().__init__(*a, **k)
fk.rns.Reticulum = Noisy
peer = fk.peer()
sys.argv = ["lxmf_sidecar.py"] + %(argv)r
runpy.run_path(%(script)r, run_name="__main__")
'''

    def launch(self, mode="client"):
        import subprocess
        import sys
        tmp = t.tempfile.TemporaryDirectory(prefix="lxmf-sidecar-proc-")
        self.addCleanup(tmp.cleanup)
        args = t.Args(tmp.name)
        code = self.RUNNER % {"tests": t.HERE, "mode": mode, "argv": args.argv(), "script": os.path.join(t.ROOT, "lxmf_sidecar.py")}
        p = subprocess.Popen([sys.executable, "-c", code], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                             stderr=subprocess.PIPE)
        self.addCleanup(lambda: p.poll() is None and p.kill())
        return p

    def test_stdout_is_only_protocol_and_sigterm_is_clean(self):
        import signal
        p = self.launch()
        first = p.stdout.readline()
        self.assertEqual(json.loads(first)["type"], "ready")
        second = p.stdout.readline()
        self.assertEqual(json.loads(second)["type"], "link")
        p.send_signal(signal.SIGTERM)
        try:
            out, err = p.communicate(timeout=20)
        except Exception:
            p.kill()
            raise
        self.assertEqual(p.returncode, 0)
        self.assertNotIn(b"NOISE", out)       # nothing but protocol lines on stdout
        self.assertIn(b"LIBRARY NOISE ON STDOUT", err)
        self.assertIn(b"RAW NOISE ON FD1", err)

    def test_quit_command_and_stdin_eof(self):
        p = self.launch()
        p.stdin.write(b'{"type":"quit"}\n')
        p.stdin.flush()
        out, err = p.communicate(timeout=20)
        self.assertEqual(p.returncode, 0)
        for line in out.splitlines():
            json.loads(line)
        p = self.launch()
        p.stdin.close()
        p.stdin = None  # communicate() must not flush the closed pipe (CPython 3.11)
        out, err = p.communicate(timeout=20)
        self.assertEqual(p.returncode, 0)

    def test_partitioned_exit_code_3_in_a_real_process(self):
        p = self.launch("instance")
        out, err = p.communicate(timeout=20)
        self.assertEqual(p.returncode, 3)
        self.assertEqual(json.loads(out.splitlines()[0]), {"type": "link", "state": "partitioned"})


if __name__ == "__main__":
    unittest.main()
