#!/usr/bin/env python3
"""7-relay/verify/stub-ntfy.py: a stdlib-only stand-in for an ntfy server (spec 7-relay/README.md
section 6.2, section 6.4, section 15 G10; row P1). It is not ntfy: it models the parts the 111
carrier uses, as upstream's docs were read from this box (R07g1 section 3; the rest is marked
"unverified from this box" in the spec), so the carrier prototype and its smoke log can run where
no real server is reachable.

Modelled:
  publish   POST|PUT /<topic>, body = the message (UTF-8, at most --message-limit bytes, else 413;
            not UTF-8 => 400, attachments are off), answer = the message as JSON (200)
  poll      GET /<topic>/json?poll=1[&since=all|<id>]  newline-delimited JSON, message events only
  held      GET /<topic>/json[?since=all|<id>]  chunked NDJSON: an "open" event, the cached
            messages after `since` (none without it), then new messages as they are published and
            a "keepalive" event every --keepalive seconds (0: never, to test a client read timeout)
  ids       12 random alphanumerics per message (upstream's shape); topics [-_A-Za-z0-9]{1,64}
  since     all | <id>; an id this stub does not hold (a restart lost the cache, or the message
            expired) answers 400 with --since-miss=error (the default: the explicit signal the
            spec's G10 item 3 asserts on) or every cached message with --since-miss=all (a silent
            fallback; which one upstream does is the section 6.4 bench item, unverified from this box)
  limiter   upstream's defaults: a burst of --burst requests per visitor (client IP), then one per
            --replenish seconds (a token bucket on a monotonic clock) => 429; at most
            --subscriptions held streams per visitor => 429
  auth      --token T (repeatable): requests without "Authorization: Bearer <T>" are refused
            (no header 403, as `auth-default-access: deny-all`; an unknown token 401)
  cache     in memory only, --cache-duration seconds (upstream's default 12 h): a restart of this
            process loses every message, which is the point
  health    GET /v1/health => {"healthy":true} (never limited)

The bound port is printed on stdout as one line (bind --port 0 for an ephemeral one), then stdout
is not written again; requests are logged on stderr unless --quiet. Stops on SIGTERM or SIGINT.
"""

import argparse
import http.server
import json
import re
import secrets
import signal
import string
import sys
import threading
import time
import urllib.parse

TOPIC_RE = re.compile(r"^[-_A-Za-z0-9]{1,64}$")
ID_RE = re.compile(r"^[A-Za-z0-9]{12}$")
ALNUM = string.ascii_letters + string.digits


def new_id():
    return "".join(secrets.choice(ALNUM) for _ in range(12))


class Store:
    """Topics, messages, the limiter and the held streams; one lock (a condition) for all."""

    def __init__(self, args):
        self.args = args
        self.cond = threading.Condition()
        self.topics = {}  # topic -> [(seq, message dict)], oldest first
        self.seq = 0  # internal order of publication (never sent)
        self.buckets = {}  # visitor -> [tokens, last monotonic]
        self.streams = {}  # visitor -> open held streams

    def allow(self, visitor):
        """Token bucket: capacity --burst, one token back per --replenish seconds."""
        now = time.monotonic()
        with self.cond:
            tokens, last = self.buckets.get(visitor, [float(self.args.burst), now])
            if self.args.replenish > 0:
                tokens = min(float(self.args.burst), tokens + (now - last) / self.args.replenish)
            if tokens < 1.0:
                self.buckets[visitor] = [tokens, now]
                return False
            self.buckets[visitor] = [tokens - 1.0, now]
            return True

    def prune(self):
        cutoff = time.time() - self.args.cache_duration
        for msgs in self.topics.values():
            while msgs and msgs[0][1]["time"] < cutoff:
                msgs.pop(0)

    def publish(self, topic, text):
        now = int(time.time())
        msg = {
            "id": new_id(),
            "time": now,
            "expires": now + int(self.args.cache_duration),
            "event": "message",
            "topic": topic,
            "message": text,
        }
        with self.cond:
            self.prune()
            self.seq += 1
            self.topics.setdefault(topic, []).append((self.seq, msg))
            self.cond.notify_all()
        return msg

    def since(self, topic, since):
        """(messages after `since` ("all" or an id), the newest internal seq), one snapshot under
        the lock, so a held stream misses nothing published in between. The list is None when the
        id is unknown (a miss)."""
        with self.cond:
            self.prune()
            msgs = [m for _, m in self.topics.get(topic, [])]
            seq = self.seq
        if since == "all":
            return msgs, seq
        for i, m in enumerate(msgs):
            if m["id"] == since:
                return msgs[i + 1 :], seq
        return None, seq

    def after(self, topic, seq):
        """(messages published after internal `seq`, the newest seq) under the lock."""
        msgs = self.topics.get(topic, [])
        return [m for s, m in msgs if s > seq], (msgs[-1][0] if msgs else seq)


def error_body(code, http_status, text):
    return json.dumps({"code": code, "http": http_status, "error": text}) + "\n"


class Handler(http.server.BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"
    server_version = "stub-ntfy"
    sys_version = ""

    # ------------------------------------------------------------ plumbing

    def log_message(self, fmt, *args):  # noqa: N802 (http.server's name)
        if not self.server.store.args.quiet:
            sys.stderr.write("stub-ntfy: %s %s\n" % (self.address_string(), fmt % args))

    def visitor(self):
        return self.client_address[0]

    def answer(self, status, body, ctype="application/json"):
        data = body.encode("utf-8")
        self.send_response(status)
        self.send_header("Content-Type", ctype)
        self.send_header("Content-Length", str(len(data)))
        self.send_header("Connection", "close")
        self.end_headers()
        self.wfile.write(data)
        self.close_connection = True

    def authorized(self):
        tokens = self.server.store.args.token
        if not tokens:
            return True
        header = self.headers.get("Authorization", "")
        if not header:
            self.answer(403, error_body(40301, 403, "forbidden"))
            return False
        if not header.startswith("Bearer ") or header[len("Bearer ") :] not in tokens:
            self.answer(401, error_body(40101, 401, "unauthorized"))
            return False
        return True

    def limited(self):
        if self.server.store.allow(self.visitor()):
            return False
        self.answer(429, error_body(42901, 429, "limit reached: too many requests"))
        return True

    def route(self):
        """(topic, json?, query) for /<topic> or /<topic>/json, else None."""
        parsed = urllib.parse.urlsplit(self.path)
        parts = [p for p in parsed.path.split("/") if p]
        query = urllib.parse.parse_qs(parsed.query, keep_blank_values=True)
        if len(parts) == 1 and TOPIC_RE.match(parts[0]):
            return parts[0], False, query
        if len(parts) == 2 and parts[1] == "json" and TOPIC_RE.match(parts[0]):
            return parts[0], True, query
        return None

    # ------------------------------------------------------------ publish

    def do_POST(self):  # noqa: N802
        self.publish()

    def do_PUT(self):  # noqa: N802
        self.publish()

    def publish(self):
        route = self.route()
        if route is None or route[1]:
            self.answer(404, error_body(40401, 404, "page not found"))
            return
        topic = route[0]
        if self.limited() or not self.authorized():
            return
        limit = self.server.store.args.message_limit
        try:
            length = int(self.headers.get("Content-Length", ""))
        except ValueError:
            self.answer(411, error_body(41101, 411, "length required"))
            return
        if length < 0:
            self.answer(400, error_body(40000, 400, "bad length"))
            return
        if length > limit:
            # read at most limit + 1 bytes of it, then refuse (attachments are off)
            self.rfile.read(min(length, limit + 1))
            self.answer(413, error_body(41301, 413, "message too large (attachments are off)"))
            return
        raw = self.rfile.read(length)
        try:
            text = raw.decode("utf-8")
        except UnicodeDecodeError:
            self.answer(400, error_body(40000, 400, "not UTF-8 (attachments are off)"))
            return
        msg = self.server.store.publish(topic, text)
        self.answer(200, json.dumps(msg) + "\n")

    # ------------------------------------------------------------ poll and held

    def do_GET(self):  # noqa: N802
        parsed = urllib.parse.urlsplit(self.path)
        if parsed.path == "/v1/health":
            self.answer(200, json.dumps({"healthy": True}) + "\n")
            return
        route = self.route()
        if route is None or not route[1]:
            self.answer(404, error_body(40401, 404, "page not found"))
            return
        topic, _, query = route
        if self.limited() or not self.authorized():
            return
        poll = query.get("poll", ["0"])[-1] in ("1", "true", "yes")
        since = query.get("since", [None])[-1]
        if since is not None and since != "all" and not ID_RE.match(since):
            self.answer(400, error_body(40008, 400, "invalid since parameter"))
            return
        store = self.server.store
        if since is None:
            msgs, seq = store.since(topic, "all")
            if not poll:
                msgs = []  # a held stream without `since` starts with new messages only
        else:
            msgs, seq = store.since(topic, since)
            if msgs is None:
                if store.args.since_miss == "error":
                    self.answer(400, error_body(40000, 400, "since: unknown message id (stub)"))
                    return
                msgs, seq = store.since(topic, "all")
        if poll:
            body = "".join(json.dumps(m) + "\n" for m in msgs)
            self.answer(200, body, "application/x-ndjson; charset=utf-8")
            return
        self.held(topic, msgs, seq)

    def chunk(self, obj):
        data = (json.dumps(obj) + "\n").encode("utf-8")
        self.wfile.write(b"%x\r\n%s\r\n" % (len(data), data))
        self.wfile.flush()

    def held(self, topic, backlog, seen):
        store = self.server.store
        visitor = self.visitor()
        with store.cond:
            if store.streams.get(visitor, 0) >= store.args.subscriptions:
                over = True
            else:
                over = False
                store.streams[visitor] = store.streams.get(visitor, 0) + 1
        if over:
            self.answer(429, error_body(42903, 429, "limit reached: too many subscriptions"))
            return
        try:
            self.send_response(200)
            self.send_header("Content-Type", "application/x-ndjson; charset=utf-8")
            self.send_header("Transfer-Encoding", "chunked")
            self.send_header("Connection", "close")
            self.end_headers()
            self.close_connection = True
            self.chunk({"id": new_id(), "time": int(time.time()), "event": "open", "topic": topic})
            for m in backlog:
                self.chunk(m)
            keepalive = store.args.keepalive
            next_keepalive = time.monotonic() + keepalive if keepalive > 0 else None
            while True:
                with store.cond:
                    fresh, newest = store.after(topic, seen)
                    if not fresh:
                        timeout = 3600.0
                        if next_keepalive is not None:
                            timeout = max(0.0, next_keepalive - time.monotonic())
                        store.cond.wait(timeout=timeout)
                        fresh, newest = store.after(topic, seen)
                seen = max(seen, newest)
                for m in fresh:
                    self.chunk(m)
                if next_keepalive is not None and time.monotonic() >= next_keepalive:
                    self.chunk(
                        {"id": new_id(), "time": int(time.time()), "event": "keepalive", "topic": topic}
                    )
                    next_keepalive = time.monotonic() + keepalive
        except (BrokenPipeError, ConnectionResetError, OSError):
            pass
        finally:
            with store.cond:
                store.streams[visitor] = max(0, store.streams.get(visitor, 1) - 1)


class Server(http.server.ThreadingHTTPServer):
    daemon_threads = True
    allow_reuse_address = True


def main():
    ap = argparse.ArgumentParser(description="stdlib stand-in for an ntfy server (7-relay P1)")
    ap.add_argument("--host", default="127.0.0.1")
    ap.add_argument("--port", type=int, default=0, help="0: an ephemeral port (printed on stdout)")
    ap.add_argument("--keepalive", type=float, default=45.0, help="seconds; 0 = never")
    ap.add_argument("--burst", type=int, default=60)
    ap.add_argument("--replenish", type=float, default=5.0, help="seconds per request token")
    ap.add_argument("--subscriptions", type=int, default=30, help="held streams per visitor")
    ap.add_argument("--message-limit", type=int, default=4096, help="bytes")
    ap.add_argument("--cache-duration", type=float, default=12 * 3600.0, help="seconds")
    ap.add_argument("--since-miss", choices=("error", "all"), default="error")
    ap.add_argument("--token", action="append", default=[])
    ap.add_argument("--quiet", action="store_true")
    args = ap.parse_args()

    server = Server((args.host, args.port), Handler)
    server.store = Store(args)

    def stop(_signum, _frame):
        raise SystemExit(0)  # interrupts serve_forever's wait; daemon threads end with the process

    signal.signal(signal.SIGTERM, stop)
    signal.signal(signal.SIGINT, stop)
    sys.stdout.write("%d\n" % server.server_address[1])
    sys.stdout.flush()
    try:
        server.serve_forever(poll_interval=3600)
    finally:
        server.server_close()


if __name__ == "__main__":
    main()
