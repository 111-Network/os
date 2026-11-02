"""Test support: a fake RNS + LXMF surface injected through sys.modules (the presence sidecar's pattern).

Nothing here imports the real libraries, opens a socket or touches the network. The fakes model only
the parts of the surface the helper uses, with the semantics read from rns 1.5.7 / lxmf 1.2.1:

* ``RNS.Reticulum(configdir, loglevel, logdest, require_shared_instance)`` and its
  ``is_connected_to_shared_instance`` flag; with ``require_shared_instance`` a would-be instance raises
  ``SystemError``.
* ``RNS.Identity`` (recall of a destination hash, ``get_public_key`` 64 B, 64 B identity files),
  ``RNS.Destination`` (OUT destinations), ``RNS.Transport`` (announce handlers, interfaces,
  ``request_path``: recorded, and answered from ``Fakes.path_table`` like a shared rnsd would).
* ``LXMF.LXMRouter`` (delivery identity, callbacks, ``handle_outbound``, ``announce``),
  ``LXMF.LXMessage`` (state constants, hash, fields, callbacks) and the field-id constants.
"""
import hashlib
import io
import os
import sys
import tempfile
import types

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
if ROOT not in sys.path:
    sys.path.insert(0, ROOT)

import lxmf_sidecar as sc  # noqa: E402  (after the path fix)


class FakeClock:
    def __init__(self, start=1000.0):
        self.now = start

    def __call__(self):
        return self.now

    def advance(self, seconds):
        self.now += seconds


class FakeIdentity:
    registry = None  # set by Fakes

    def __init__(self, create_keys=True, seed=None):
        self.prv = seed if seed is not None else os.urandom(64)
        self.pub = hashlib.sha512(self.prv).digest()  # a stand-in public key, 64 bytes

    @classmethod
    def from_file(cls, path):
        try:
            with open(path, "rb") as f:
                data = f.read()
        except OSError:
            return None
        return cls(seed=data) if len(data) == 64 else None

    def to_file(self, path):
        with open(path, "wb") as f:
            f.write(self.prv)

    def get_public_key(self):
        return self.pub

    def get_private_key(self):
        return self.prv

    def dest_hash(self):
        return hashlib.sha256(self.pub + b"lxmf.delivery").digest()[:16]


class FakeDestination:
    OUT, IN, SINGLE = 0x02, 0x01, 0x00

    def __init__(self, identity, direction, kind, app, *aspects):
        self.identity = identity
        self.direction = direction
        self.hash = identity.dest_hash()
        self.display_name = None
        self.type = kind


class FakeMessage:
    """LXMF.LXMessage as far as the helper uses it."""

    GENERATING, OUTBOUND, SENDING, SENT, DELIVERED = 0x00, 0x01, 0x02, 0x04, 0x08
    REJECTED, CANCELLED, FAILED = 0xFD, 0xFE, 0xFF
    OPPORTUNISTIC, DIRECT, PROPAGATED = 0x01, 0x02, 0x03
    SOURCE_UNKNOWN, SIGNATURE_INVALID = 0x01, 0x02

    def __init__(self, destination, source, content="", title="", fields=None, desired_method=None, **kw):
        self.destination = destination
        self.source = source
        self.destination_hash = destination.hash if destination else None
        self.source_hash = source.hash if source else None
        self.content = content.encode("utf-8") if isinstance(content, str) else content
        self.fields = fields if fields is not None else {}
        self.desired_method = desired_method
        self.method = desired_method or self.DIRECT
        self.state = self.GENERATING
        self.hash = None
        self.timestamp = None
        self.signature_validated = False
        self.unverified_reason = None
        self.delivered_cb = None
        self.failed_cb = None

    def register_delivery_callback(self, cb):
        self.delivered_cb = cb

    def register_failed_callback(self, cb):
        self.failed_cb = cb

    # ---- test drivers: what the library does, in the library's order
    def lib_set(self, state):
        self.state = state

    def lib_deliver(self):
        self.state = self.DELIVERED
        if self.delivered_cb:
            self.delivered_cb(self)

    def lib_propagated(self):
        self.state = self.SENT
        self.method = self.PROPAGATED
        if self.delivered_cb:
            self.delivered_cb(self)

    def lib_fail(self, state=None):
        self.state = state if state is not None else self.FAILED
        if self.failed_cb:
            self.failed_cb(self)


class FakeRouter:
    def __init__(self, identity=None, storagepath=None, **kw):
        self.identity = identity
        self.storagepath = storagepath
        self.delivery_cb = None
        self.dest = None
        self.outbound = []
        self.announced = []
        self.exit_called = False
        self._n = 0

    def register_delivery_identity(self, identity, display_name=None, stamp_cost=None):
        self.stamp_cost = stamp_cost
        self.dest = FakeDestination(identity, FakeDestination.IN, FakeDestination.SINGLE, "lxmf", "delivery")
        self.dest.display_name = display_name
        return self.dest

    def register_delivery_callback(self, cb):
        self.delivery_cb = cb

    def announce(self, destination_hash, attached_interface=None):
        self.announced.append((bytes(destination_hash), self.dest.display_name))

    def handle_outbound(self, m):
        self._n += 1
        m.state = FakeMessage.OUTBOUND
        m.hash = hashlib.sha256(m.destination_hash + m.source_hash + m.content + self._n.to_bytes(4, "big")).digest()
        self.outbound.append(m)

    def exit_handler(self):
        self.exit_called = True


class FakeInterface:
    def __init__(self, online=True):
        self.online = online


class LocalClientInterface(FakeInterface):
    """Named like the real class: the helper finds the instance link by this name."""


class FakeTransport:
    def __init__(self):
        self.handlers = []
        self.interfaces = []
        self.path_requests = []  # every RNS.Transport.request_path(destination_hash) call, in order
        self.on_request = None   # what the shared instance does with one (set by Fakes)

    def register_announce_handler(self, handler):
        if hasattr(handler, "aspect_filter") and callable(handler.received_announce):
            self.handlers.append(handler)

    def request_path(self, destination_hash, on_interface=None, tag=None, recursive=False):
        """RNS.Transport.request_path (rns 1.5.7 signature). A shared-instance client's request goes to
        its rnsd, which answers from its path table with the cached announce (a path response)."""
        self.path_requests.append(bytes(destination_hash))
        if self.on_request is not None:
            self.on_request(bytes(destination_hash))


class Fakes:
    """One installed fake surface. ``mode``: ``client`` (an instance is serving), ``instance`` (none is,
    so Reticulum() would become the instance), ``old_instance`` (RNS without require_shared_instance)."""

    def __init__(self, mode="client"):
        self.mode = mode
        self.transport = FakeTransport()
        self.transport.interfaces.append(LocalClientInterface(True))
        self.known = {}      # destination hash -> FakeIdentity
        self.app_data = {}   # destination hash -> bytes
        self.path_table = {}  # destination hash -> (FakeIdentity, name): what the shared rnsd knows
        self.transport.on_request = self._answer_path_request
        self.reticulum_calls = []
        self.exit_handler_ran = False
        fakes = self

        class Identity(FakeIdentity):
            @staticmethod
            def recall(dest_hash, **kw):
                return fakes.known.get(bytes(dest_hash))

            @staticmethod
            def recall_app_data(dest_hash, **kw):
                return fakes.app_data.get(bytes(dest_hash))

        if mode == "old_instance":
            class Reticulum:
                def __init__(self, configdir=None, loglevel=None, logdest=None):
                    fakes.reticulum_calls.append({"configdir": configdir, "loglevel": loglevel})
                    self.is_connected_to_shared_instance = False

                @staticmethod
                def exit_handler():
                    fakes.exit_handler_ran = True
        else:
            class Reticulum:
                def __init__(self, configdir=None, loglevel=None, logdest=None, require_shared_instance=False):
                    fakes.reticulum_calls.append({"configdir": configdir, "loglevel": loglevel, "logdest": logdest,
                                                  "require_shared_instance": require_shared_instance})
                    if fakes.mode == "instance":
                        if require_shared_instance:
                            raise SystemError("no shared instance available")
                        self.is_connected_to_shared_instance = False
                    else:
                        self.is_connected_to_shared_instance = True

                @staticmethod
                def exit_handler():
                    fakes.exit_handler_ran = True

        rns = types.ModuleType("RNS")
        rns.Reticulum = Reticulum
        rns.Identity = Identity
        rns.Destination = FakeDestination
        rns.Transport = self.transport
        rns.LOG_WARNING, rns.LOG_INFO = 2, 4
        lx = types.ModuleType("LXMF")
        lx.LXMRouter = FakeRouter
        lx.LXMessage = FakeMessage
        lx.FIELD_CUSTOM_TYPE, lx.FIELD_CUSTOM_DATA = 0xFB, 0xFC
        lx.display_name_from_app_data = lambda ad: None if not ad else ad.decode("utf-8")
        self.rns, self.lxmf = rns, lx
        self.Identity = Identity

    # ---- the world
    def peer(self, announced=True):
        """A remote identity; with ``announced`` the library already knows it (recall works)."""
        ident = self.Identity()
        if announced:
            self.known[ident.dest_hash()] = ident
        return ident

    def announce(self, ident, name="Alice"):
        """The peer announces: remembered, then the helper's handler runs (as RNS does)."""
        d = ident.dest_hash()
        self.known[d] = ident
        data = name.encode("utf-8") if name is not None else None
        self.app_data[d] = data
        for h in list(self.transport.handlers):
            h.received_announce(d, ident, data)
        return d

    def _answer_path_request(self, d):
        """rnsd answers a local client's path request from its table (even with transport off): the
        cached announce reaches the client's announce handlers as a path response."""
        if d in self.path_table:
            ident, name = self.path_table[d]
            self.announce(ident, name)

    def install(self):
        self._saved = {k: sys.modules.get(k) for k in ("RNS", "LXMF")}
        sys.modules["RNS"] = self.rns
        sys.modules["LXMF"] = self.lxmf
        return self

    def uninstall(self):
        for k, v in self._saved.items():
            if v is None:
                sys.modules.pop(k, None)
            else:
                sys.modules[k] = v


class Rec(io.BytesIO):
    """A protocol stream that counts flushes and can be parsed into lines."""

    def __init__(self):
        io.BytesIO.__init__(self)
        self.flushes = 0

    def flush(self):
        self.flushes += 1

    def lines(self):
        data = self.getvalue()
        return [] if not data else data.decode("utf-8").split("\n")[:-1]

    def objs(self):
        import json
        return [json.loads(l) for l in self.lines()]


class Args:
    def __init__(self, root, name=None, announce_interval=60, verbose=False):
        self.config_dir = os.path.join(root, "rns")
        self.identity = os.path.join(root, "identity")
        self.storage = os.path.join(root, "store")
        self.name = name
        self.announce_interval = announce_interval
        self.verbose = verbose

    def argv(self):
        a = ["--config-dir", self.config_dir, "--identity", self.identity, "--storage", self.storage,
             "--announce-interval", str(self.announce_interval)]
        if self.name:
            a += ["--name", self.name]
        if self.verbose:
            a.append("-v")
        return a


class World:
    """A started helper over fakes, with a fake monotonic clock and captured streams."""

    def __init__(self, mode="client", name=None, announce_interval=60, start=True, config=None):
        self.tmp = tempfile.TemporaryDirectory(prefix="lxmf-sidecar-test-")
        self.root = self.tmp.name
        self.fakes = Fakes(mode).install()
        self.clock = FakeClock()
        self.stdout = Rec()
        self.stderr = io.StringIO()
        self.args = Args(self.root, name, announce_interval)
        if config is not None:
            os.makedirs(self.args.config_dir, mode=0o700)
            with open(os.path.join(self.args.config_dir, "config"), "w") as f:
                f.write(config)
        self.log = sc.Log(self.stderr, False)
        self.out = sc.Emitter(self.stdout, self.log)
        self.helper = sc.Helper(self.args, self.fakes.rns, self.fakes.lxmf, self.out, self.log, self.clock)
        if start:
            self.helper.attach()
            self.helper.start()

    @property
    def router(self):
        return self.helper.router

    def cmd(self, obj):
        import json
        raw = obj if isinstance(obj, bytes) else json.dumps(obj).encode("utf-8")
        return self.helper.handle_line(raw + b"\n")

    def send(self, peer, sid="m1", text="hello", fields=None):
        c = {"type": "send", "id": sid, "to": peer.dest_hash().hex(), "text": text}
        if fields is not None:
            c["fields"] = fields
        return self.cmd(c)

    def states(self, sid=None):
        return [o for o in self.stdout.objs() if o["type"] == "state" and (sid is None or o["id"] == sid)]

    def of(self, kind):
        return [o for o in self.stdout.objs() if o["type"] == kind]

    def close(self):
        self.fakes.uninstall()
        self.tmp.cleanup()


def inbound(world, peer=None, content=b"hi", fields=None, signed=True, reason=None, ts=1700000000.5, h=None):
    """An inbound LXMessage as the router's delivery callback receives it."""
    peer = peer or world.fakes.peer()
    src = peer.dest_hash()
    m = FakeMessage(FakeDestination(world.helper.identity, 0, 0, "lxmf", "delivery"),
                    FakeDestination(peer, 0, 0, "lxmf", "delivery"), content, fields=fields)
    m.hash = h or hashlib.sha256(src + content + repr(sorted((fields or {}).items(), key=str)).encode()).digest()
    m.timestamp = ts
    m.signature_validated = signed
    m.unverified_reason = reason
    return m
