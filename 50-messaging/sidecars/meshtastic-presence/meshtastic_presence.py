#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
#
# meshtastic-presence: read-only Meshtastic presence reader for the 111network OS.
# Copyright (C) 2026 the 111network authors
#
# This program is free software: you can redistribute it and/or modify it under
# the terms of the GNU General Public License as published by the Free Software
# Foundation, either version 3 of the License, or (at your option) any later
# version. It is distributed in the hope that it will be useful, but WITHOUT ANY
# WARRANTY; without even the implied warranty of MERCHANTABILITY or FITNESS FOR A
# PARTICULAR PURPOSE. See the GNU General Public License (file LICENSE next to this
# file) for more details.
#
# This program imports meshtastic-python (GPL-3.0-only), which is why it is a GPL
# program in its own directory, run as a separate process in its own hash-locked venv.
# No part of it is linked into the 111 binaries (spec 50-messaging/README.md section 14,
# "GPL boundary").
"""meshtastic-presence: read-only presence reader (spec section 14).

Connects to ONE Meshtastic TCP client-API endpoint (meshmud's TCP downlink,
``127.0.0.1:11230``), asks the node for its configuration once (``want_config``),
listens to NodeInfo / position / telemetry events through meshtastic-python's
pubsub bus, and writes ``<out>/meshtastic.json`` atomically::

    {"schema": 1, "nodes": [{"id", "num", "long_name", "short_name", "pubkey"?,
     "snr"?, "rssi"?, "hops_away"?, "last_heard_hint"?, "position"?,
     "channel_utilization"?}]}

Hard rules (spec section 14, "Presence reader"):

* **No send path, except the session keep-alive.** 111 never transmits on the Meshtastic
  mesh and never changes node configuration. Exactly two ``ToRadio`` messages may leave
  this process (:func:`to_radio_kind`, :func:`allowed_to_radio`): the single
  ``want_config_id`` of a connection, and a ``heartbeat`` with nonce 0 every
  :data:`KEEPALIVE_INTERVAL` (5 minutes), which keeps the board's client-API session open
  and makes the board do nothing on the mesh. Text, data, admin, position, traceroute
  packets, a heartbeat with any other nonce (nonce 1 makes the board broadcast its
  NodeInfo, a transmit), the library's own heartbeat timer and its ``disconnect`` message
  (which meshmud would forward to the board and which would end the board's API session
  for every other client) are all refused at the lowest layer
  (:class:`ReadOnlyTCPInterface`). ``tests/test_presence.py`` proves it by AST scan and by
  behaviour.
* **Untrusted strings.** Everything from the radio is RF-origin: names are stripped of
  control and bidi-override characters and cut to 40 UTF-8 bytes, ids are rebuilt from
  the node number, numeric fields are range-checked, anything odd is dropped (never
  raised).
* **Bounded output.** At most 4096 nodes and at most ``MAX_FILE_BYTES`` bytes, so the
  daemon's ingest limits (regular file, 1 MiB, 4096 nodes) always hold.
* **Atomic.** Temp file in the output directory, fsync, rename, fsync of the directory.
* **No wall clock.** Nothing here orders or authenticates by time; ``last_heard_hint``
  is the node's own clock, passed through for display only. Back-off and coalescing use
  the monotonic clock.

``--once`` is the identify mode: connect, print ``{"schema": 1, "my_node_info": {...}}``
as one JSON line on stdout, exit. It never writes the presence file.
"""
from __future__ import annotations

import argparse
import base64
import binascii
import json
import logging
import math
import os
import random
import signal
import socket
import struct
import sys
import tempfile
import threading
import time
import unicodedata
from typing import Any, Callable, Dict, Iterable, List, Optional, Tuple

__version__ = "1.0.0"

LOG = logging.getLogger("meshtastic-presence")

SCHEMA = 1
OUT_NAME = "meshtastic.json"
DEFAULT_ENDPOINT = "127.0.0.1:11230"
MAX_NODES = 4096
MAX_NAME_BYTES = 40
#: The daemon accepts a regular file of at most 1 MiB (1_048_576); stay clearly below.
MAX_FILE_BYTES = 1_000_000
BROADCAST_NUM = 0xFFFFFFFF

#: ToRadio ``payload_variant`` names this program may emit: ``want_config_id`` (once per
#: connection) and ``heartbeat`` (only the empty, nonce-0 one; see :func:`to_radio_kind`).
ALLOWED_TORADIO = frozenset({"want_config_id", "heartbeat"})

# Why a keep-alive at all (Layer 6 research finding, Meshtastic firmware v2.7.26): the board
# closes its client-API session after 15 minutes without ANY ToRadio frame, serial and TCP
# alike (src/SerialConsole.cpp:27,119-121 and src/mesh/api/ServerAPI.cpp:6,34), and FromRadio
# stops until a new want_config_id. meshmud forwards every downlink ToRadio verbatim to the
# board and originates nothing but one want_config at uplink connect (internal/common/
# uplink.go:95-106); the frames this reader sends count as ToRadio on the board, the link
# between reader and meshmud does not. So with no web panel open the board went silent after
# 15 minutes and Nearby froze. Sending one nonce-0 heartbeat every 5 minutes (the Rust client
# and the web SDK do the same) restarts the board's timer three times per window. Nonce 1 is
# the one value that makes the board broadcast its NodeInfo on the mesh (src/mesh/PhoneAPI.cpp,
# heartbeat case): a transmit, so the guard below refuses every heartbeat whose nonce is not 0.
#: Seconds between session keep-alive heartbeats, on the monotonic clock.
KEEPALIVE_INTERVAL = 300.0

#: Seconds a keep-alive may wait for meshmud to take its bytes (a write timeout on the
#: socket, see :func:`set_write_timeout`). A meshmud that stopped reading would otherwise
#: block the library's ``sendall``, and with it the reader loop, for good. A keep-alive
#: that times out ends the session like a read failure: the link is reported lost and the
#: reader reconnects with back-off.
KEEPALIVE_WRITE_TIMEOUT = 5.0

#: Format and bidi-override characters that are not Unicode category Cc but can still
#: reorder or hide text when an RF-origin name is rendered.
_BIDI_AND_INVISIBLE = frozenset(
    [0x061C, 0x200E, 0x200F, 0x2060, 0xFEFF]
    + list(range(0x202A, 0x202F))
    + list(range(0x2066, 0x206A))
)
_DROP_CATEGORIES = frozenset({"Cc", "Cs", "Zl", "Zp"})


# --------------------------------------------------------------------------- sanitizing
def sanitize_text(value: Any, max_bytes: int = MAX_NAME_BYTES) -> str:
    """Return ``value`` as a safe display string of at most ``max_bytes`` UTF-8 bytes.

    Control characters (Cc), surrogates, line/paragraph separators and bidi overrides are
    removed; the result is trimmed, cut on a code point boundary and trimmed again.
    Anything that is not a ``str`` becomes the empty string.
    """
    if not isinstance(value, str):
        return ""
    kept = []
    for ch in value:
        if ord(ch) in _BIDI_AND_INVISIBLE or unicodedata.category(ch) in _DROP_CATEGORIES:
            continue
        kept.append(ch)
    text = "".join(kept).strip()
    raw = text.encode("utf-8", errors="ignore")[:max_bytes]
    return raw.decode("utf-8", errors="ignore").strip()


def _as_int(value: Any) -> Optional[int]:
    """``value`` as an int when it already is one (bools and floats are refused)."""
    if isinstance(value, bool) or not isinstance(value, int):
        return None
    return value


def _as_float(value: Any) -> Optional[float]:
    """``value`` as a finite float when it is a real number (bools refused)."""
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        return None
    f = float(value)
    return f if math.isfinite(f) else None


def node_id_for(num: int) -> str:
    """The canonical Meshtastic node id for ``num``: ``!`` plus 8 lowercase hex digits."""
    return "!%08x" % num


def _pubkey_b64(value: Any) -> Optional[str]:
    """Canonical base64 of a 32-byte X25519 public key, or None when it is not one."""
    if not isinstance(value, str) or len(value) > 64:
        return None
    try:
        raw = base64.b64decode(value.encode("ascii"), validate=True)
    except (binascii.Error, UnicodeEncodeError, ValueError):
        return None
    if len(raw) != 32:
        return None
    return base64.b64encode(raw).decode("ascii")


def _dict(value: Any) -> Dict[str, Any]:
    return value if isinstance(value, dict) else {}


def _position(raw: Any) -> Optional[Dict[str, Any]]:
    """``{lat, lon, alt?}`` from a node ``position`` dict, or None without a usable fix."""
    pos = _dict(raw)
    lat_i, lon_i = _as_int(pos.get("latitudeI")), _as_int(pos.get("longitudeI"))
    if lat_i is not None and lon_i is not None:
        lat, lon = lat_i / 1e7, lon_i / 1e7
    else:
        flat, flon = _as_float(pos.get("latitude")), _as_float(pos.get("longitude"))
        if flat is None or flon is None:
            return None
        lat, lon = flat, flon
    if not (-90.0 <= lat <= 90.0 and -180.0 <= lon <= 180.0):
        return None
    if lat == 0.0 and lon == 0.0:  # "no fix" is reported as 0,0
        return None
    out: Dict[str, Any] = {"lat": round(lat, 7), "lon": round(lon, 7)}
    alt = _as_int(pos.get("altitude"))
    if alt is not None and -1000 <= alt <= 100000:
        out["alt"] = alt
    return out


def normalize_node(num: Any, node: Any) -> Optional[Dict[str, Any]]:
    """One presence row from a meshtastic-python node dict, or None when unusable.

    Never raises on odd input. Field order is fixed so identical input gives identical
    bytes.
    """
    n = _as_int(num)
    if n is None or n <= 0 or n >= BROADCAST_NUM:
        return None
    nd = _dict(node)
    user = _dict(nd.get("user"))
    row: Dict[str, Any] = {
        "id": node_id_for(n),
        "num": n,
        "long_name": sanitize_text(user.get("longName")),
        "short_name": sanitize_text(user.get("shortName")),
    }
    pubkey = _pubkey_b64(user.get("publicKey"))
    if pubkey is not None:
        row["pubkey"] = pubkey
    snr = _as_float(nd.get("snr"))
    if snr is not None and -100.0 <= snr <= 100.0:
        row["snr"] = round(snr, 2)
    rssi = _as_int(_dict(nd.get("lastReceived")).get("rxRssi"))
    if rssi is not None and -200 <= rssi <= 0:
        row["rssi"] = rssi
    hops = _as_int(nd.get("hopsAway"))
    if hops is not None and 0 <= hops <= 7:
        row["hops_away"] = hops
    heard = _as_int(nd.get("lastHeard"))
    if heard is not None and 0 < heard <= 0xFFFFFFFF:
        row["last_heard_hint"] = heard
    position = _position(nd.get("position"))
    if position is not None:
        row["position"] = position
    util = _as_float(_dict(nd.get("deviceMetrics")).get("channelUtilization"))
    if util is not None and 0.0 <= util <= 100.0:
        row["channel_utilization"] = round(util, 2)
    return row


def normalize_nodes(nodes_by_num: Any, skip_num: Optional[int] = None) -> List[Dict[str, Any]]:
    """Rows for every usable node in a ``nodesByNum``-shaped mapping (own node skipped)."""
    rows: List[Dict[str, Any]] = []
    if not isinstance(nodes_by_num, dict):
        return rows
    for num, node in _copy_items(nodes_by_num):
        if skip_num is not None and num == skip_num:
            continue
        row = normalize_node(num, node)
        if row is not None:
            rows.append(row)
    return rows


def _copy_items(mapping: Dict[Any, Any]) -> List[Tuple[Any, Any]]:
    """Snapshot ``mapping`` and one nesting level of its dict values.

    The library mutates its node DB from another thread; each ``list(dict.items())`` /
    ``dict(d)`` is a single C-level step under the GIL, so no "changed size during
    iteration" can happen. A rare ``RuntimeError`` is retried.
    """
    for _ in range(3):
        try:
            items = list(mapping.items())
            out: List[Tuple[Any, Any]] = []
            for key, val in items:
                if isinstance(val, dict):
                    val = dict(val)
                    for sub in ("user", "position", "deviceMetrics", "lastReceived"):
                        if isinstance(val.get(sub), dict):
                            val[sub] = dict(val[sub])
                out.append((key, val))
            return out
        except RuntimeError:
            continue
    return []


# --------------------------------------------------------------------------- document
def encode_document(rows: Iterable[Dict[str, Any]]) -> Tuple[bytes, int]:
    """Serialize rows as the presence document; returns ``(bytes, dropped_count)``.

    Keeps at most ``MAX_NODES`` rows and at most ``MAX_FILE_BYTES`` bytes. When rows have
    to go, the least recently heard (and then the highest node number) are dropped first.
    The kept rows are written in ascending node-number order.
    """
    ranked = sorted(rows, key=lambda r: (-(r.get("last_heard_hint") or 0), r["num"]))
    total = len(ranked)
    keep = ranked[:MAX_NODES]

    def dump(sel: List[Dict[str, Any]]) -> bytes:
        ordered = sorted(sel, key=lambda r: r["num"])
        doc = {"schema": SCHEMA, "nodes": ordered}
        text = json.dumps(doc, ensure_ascii=False, separators=(",", ":"), allow_nan=False)
        return (text + "\n").encode("utf-8")

    data = dump(keep)
    while len(data) > MAX_FILE_BYTES and keep:
        avg = max(1, len(data) // max(1, len(keep)))
        over = len(data) - MAX_FILE_BYTES
        drop = max(1, min(len(keep), -(-over // avg)))
        keep = keep[: len(keep) - drop]
        data = dump(keep)
    return data, total - len(keep)


def write_atomic(directory: str, name: str, data: bytes) -> None:
    """Write ``data`` to ``directory/name`` via temp file, fsync and rename (mode 0644)."""
    fd, tmp = tempfile.mkstemp(prefix="." + name + ".", suffix=".tmp", dir=directory)
    try:
        with os.fdopen(fd, "wb") as handle:
            handle.write(data)
            handle.flush()
            os.fsync(handle.fileno())
            os.fchmod(handle.fileno(), 0o644)
        os.replace(tmp, os.path.join(directory, name))
    except BaseException:
        try:
            os.unlink(tmp)
        except OSError:
            pass
        raise
    try:
        dfd = os.open(directory, os.O_RDONLY)
    except OSError:
        return
    try:
        os.fsync(dfd)
    except OSError:
        pass
    finally:
        os.close(dfd)


# --------------------------------------------------------------------------- my_node_info
def _attr(obj: Any, name: str) -> Any:
    try:
        return getattr(obj, name, None)
    except Exception:  # a hostile or half-built object must not take the reader down
        return None


def build_my_node_info(iface: Any) -> Optional[Dict[str, Any]]:
    """The ``my_node_info`` object for ``--once`` (identify), or None when not known yet.

    Shape (not pinned by the spec, documented in the README)::

        {"num", "id", "long_name", "short_name", "hw_model", "firmware_version",
         "pio_env"?, "pubkey"?}
    """
    info = _attr(iface, "myInfo")
    num = _as_int(_attr(info, "my_node_num"))
    if num is None or num <= 0 or num >= BROADCAST_NUM:
        return None
    node = _dict(_dict(_attr(iface, "nodesByNum")).get(num))
    user = _dict(node.get("user"))
    meta = _attr(iface, "metadata")
    out: Dict[str, Any] = {
        "num": num,
        "id": node_id_for(num),
        "long_name": sanitize_text(user.get("longName")),
        "short_name": sanitize_text(user.get("shortName")),
        "hw_model": sanitize_text(user.get("hwModel")),
        "firmware_version": sanitize_text(_attr(meta, "firmware_version")),
    }
    pio = sanitize_text(_attr(info, "pio_env"))
    if pio:
        out["pio_env"] = pio
    pubkey = _pubkey_b64(user.get("publicKey"))
    if pubkey is not None:
        out["pubkey"] = pubkey
    return out


# --------------------------------------------------------------------------- back-off
class Backoff:
    """Exponential back-off with jitter: ``initial * factor**n`` capped at ``maximum``."""

    def __init__(self, initial: float = 1.0, factor: float = 2.0, maximum: float = 60.0,
                 jitter: float = 0.2, rng: Optional[Callable[[], float]] = None) -> None:
        self.initial, self.factor, self.maximum, self.jitter = initial, factor, maximum, jitter
        self._rng = rng or random.random
        self._n = 0

    def next(self) -> float:
        base = min(self.maximum, self.initial * (self.factor ** self._n))
        self._n += 1
        # jitter only ever shortens the wait, so the cap is a hard cap
        return base * (1.0 - self.jitter * self._rng())

    def reset(self) -> None:
        self._n = 0


# --------------------------------------------------------------------------- the interface
def _is_empty_heartbeat(heartbeat: Any) -> bool:
    """True for a ``Heartbeat`` whose nonce is 0 and which carries nothing else.

    Nonce 0 is the protobuf default and is not even put on the wire; it only restarts the
    board's idle timer. Nonce 1 makes the board broadcast its NodeInfo on the mesh (a
    transmit), so every other nonce is refused. ``ByteSize() == 0`` pins the wire form too:
    a field this library version does not know about is not an empty keep-alive either.
    """
    value = getattr(heartbeat, "nonce", 0)
    return type(value) is int and value == 0 and heartbeat.ByteSize() == 0


def to_radio_kind(message: Any) -> Optional[str]:
    """The payload variant of ``message`` when this program may send it, else None.

    ``want_config_id`` passes (the interface lets it out once per connection); ``heartbeat``
    passes only when it is empty, i.e. nonce 0; every other variant, and anything that is not
    a ToRadio, is refused. Never raises.
    """
    try:
        variant = message.WhichOneof("payload_variant")
        if variant not in ALLOWED_TORADIO:
            return None
        if variant == "heartbeat" and not _is_empty_heartbeat(message.heartbeat):
            return None
        return str(variant)
    except Exception:
        return None


def allowed_to_radio(message: Any) -> bool:
    """True only for a ``ToRadio`` that :func:`to_radio_kind` lets through."""
    return to_radio_kind(message) is not None


def set_write_timeout(sock: Any, seconds: float) -> None:
    """Bound every blocking write on ``sock`` to ``seconds`` (``SO_SNDTIMEO``); reads still block.

    ``settimeout`` would bound the library reader thread's ``recv`` on the same socket too and
    end a link that is merely quiet. A write that times out raises ``OSError`` (EAGAIN); so
    does a socket that cannot take the option. The kernel's ``struct timeval`` is sized by
    reading the current value back: two longs (32-bit time) or two 64-bit fields. Zero would
    mean "no timeout", so the bound never drops below a millisecond.
    """
    seconds = max(0.001, float(seconds))
    size = len(sock.getsockopt(socket.SOL_SOCKET, socket.SO_SNDTIMEO, 16))
    layout = {16: "qq", 8: "ii"}.get(size)
    if layout is None:
        raise OSError(f"unexpected SO_SNDTIMEO size {size}; the keep-alive write cannot be bounded")
    whole = int(seconds)
    micros = min(999_999, int(round((seconds - whole) * 1_000_000)))
    sock.setsockopt(socket.SOL_SOCKET, socket.SO_SNDTIMEO, struct.pack(layout, whole, micros))


def make_interface_class(connect_wait: float = 30.0, write_timeout: float = KEEPALIVE_WRITE_TIMEOUT) -> Any:
    """Build the read-only TCP interface class (imports meshtastic-python lazily).

    ``connect_wait`` is how long the constructor waits for the node database;
    ``write_timeout`` bounds each keep-alive write (:data:`KEEPALIVE_WRITE_TIMEOUT`).

    A function, not a module-level class, so the unit tests and the lock tooling can
    import this file without the library installed.
    """
    from meshtastic.protobuf import mesh_pb2  # type: ignore[import-not-found]
    from meshtastic.tcp_interface import TCPInterface  # type: ignore[import-not-found]

    class ReadOnlyTCPInterface(TCPInterface):  # type: ignore[misc]
        """``TCPInterface`` whose only ToRadio traffic is one ``want_config`` per connection
        and the nonce-0 session keep-alive.

        Overrides (all of them remove behaviour, bar the one keep-alive helper):

        * ``_sendToRadio`` and ``_sendToRadioImpl``: refuse every ToRadio but the first
          ``want_config_id`` and a heartbeat with nonce 0 (both the public entry and the
          implementation layer are guarded, so a library path that bypasses ``_sendToRadio``
          still cannot send). The checks run under one re-entrant lock: the keep-alive goes
          out from the reader's thread, the first ``want_config`` from the constructor's.
        * ``session_keepalive``: the one added method; builds the empty ``Heartbeat``
          ``ToRadio`` itself (nonce never assigned, so 0) and passes it through the guard,
          with the socket's writes bounded by ``write_timeout``.
          :class:`PresenceReader` calls it from its loop every :data:`KEEPALIVE_INTERVAL`.
        * ``_sendDisconnect`` / ``_startHeartbeat``: no-ops. meshmud forwards downlink frames
          to the board verbatim; a ``disconnect`` would end the board's API session for
          everybody. The library's own heartbeat timer thread is off: the keep-alive is not
          pointless on this link (the board's 15-minute idle timer counts the frames it
          receives through meshmud, see the note at :data:`KEEPALIVE_INTERVAL`) but it is
          driven from the reader loop on the monotonic clock instead of a second thread.
        * ``connect``: skips the 32 resync bytes meant for sleeping serial devices.
        * ``_reconnect``: never re-dials (and so never re-sends ``want_config``); the link
          is reported lost and :class:`PresenceReader` reconnects with back-off. It also wakes
          the library's reader thread out of a blocking ``recv`` before closing the socket
          (``_socket_shutdown``, both directions): after a keep-alive write timed out the peer
          is alive but silent, a plain close does not wake a ``recv`` on Linux, and ``close``
          of this interface waits for that thread with no timeout.
        """

        _want_config_sent = False
        _connect_wait = float(connect_wait)
        _write_timeout = float(write_timeout)
        dropped_to_radio = 0
        keepalives_sent = 0
        _impl_ok = False
        _guard_lock = threading.RLock()

        def _sendToRadio(self, toRadio: Any) -> None:  # noqa: N802 (library name)
            with self._guard_lock:
                kind = to_radio_kind(toRadio)
                if kind is None or (kind == "want_config_id" and self._want_config_sent):
                    self.dropped_to_radio += 1
                    LOG.debug("refused a ToRadio message (read-only reader)")
                    return
                if kind == "want_config_id":
                    self._want_config_sent = True
                self._impl_ok = True
                try:
                    super()._sendToRadio(toRadio)
                finally:
                    self._impl_ok = False
                if kind == "heartbeat":
                    self.keepalives_sent += 1

        def _sendToRadioImpl(self, toRadio: Any) -> None:  # noqa: N802 (library name)
            # The implementation layer is reachable only through the guarded
            # entry above; anything else (a library internal, a future code path)
            # is refused and counted. Even then the message is checked again.
            with self._guard_lock:
                if not self._impl_ok or to_radio_kind(toRadio) is None:
                    self.dropped_to_radio += 1
                    LOG.debug("refused a ToRadio message at the implementation layer (read-only reader)")
                    return
                super()._sendToRadioImpl(toRadio)

        def session_keepalive(self) -> bool:
            """Put one nonce-0 heartbeat on the wire; True when it went out.

            False when there is no socket or the guard refused it. The socket's writes are
            bounded by ``write_timeout`` first (:func:`set_write_timeout`), so a meshmud that
            stopped reading makes the library's ``sendall`` raise instead of blocking. A
            socket error or that timeout propagates (the library has already reported the
            link lost through ``_reconnect``); the caller treats it as a lost link.
            """
            with self._guard_lock:
                sock = getattr(self, "socket", None)
                if sock is None:
                    return False
                set_write_timeout(sock, self._write_timeout)
                before = self.keepalives_sent
                message = mesh_pb2.ToRadio()
                message.heartbeat.SetInParent()  # an empty Heartbeat: nonce stays 0
                self._sendToRadio(message)
                return self.keepalives_sent > before

        def _sendDisconnect(self) -> None:  # noqa: N802
            return None

        def _startHeartbeat(self) -> None:  # noqa: N802
            return None

        def connect(self) -> None:
            self.myConnect()
            self._rxThread.start()
            self._startConfig()
            if not self.noProto:
                self._waitConnected(self._connect_wait)

        def _reconnect(self) -> None:
            self._wantExit = True
            sock = self.socket
            if sock is not None:
                try:
                    self._socket_shutdown()  # wakes the reader thread's recv (see above)
                except OSError:
                    pass
                try:
                    sock.close()
                except OSError:
                    pass
            self.socket = None
            self._disconnected()

    # The overrides rely on private library names; fail loudly at class build
    # (not at first send) if the pinned library ever renames one of them.
    for _name in ("_sendToRadio", "_sendToRadioImpl", "_sendDisconnect", "_startHeartbeat", "_reconnect", "connect",
                  "_socket_shutdown"):
        if not hasattr(TCPInterface, _name):
            raise RuntimeError(f"meshtastic-python no longer defines TCPInterface.{_name}; the read-only guard cannot be trusted")
    if not hasattr(mesh_pb2.ToRadio(), "heartbeat"):
        raise RuntimeError("meshtastic-python's ToRadio has no heartbeat field; the session keep-alive cannot be built")
    return ReadOnlyTCPInterface


def default_connector(host: str, port: int, timeout: float,
                      write_timeout: float = KEEPALIVE_WRITE_TIMEOUT) -> Any:
    """Open the read-only interface; blocks until the node DB has been downloaded."""
    cls = make_interface_class(connect_wait=max(1.0, timeout), write_timeout=write_timeout)
    return cls(host, portNumber=port, connectNow=True, timeout=int(max(1, timeout)))


class PubsubBus:
    """Thin wrapper over ``pubsub.pub`` so tests can substitute a fake bus."""

    def __init__(self) -> None:
        from pubsub import pub  # type: ignore[import-not-found]

        self._pub = pub

    def subscribe(self, listener: Callable[..., None], topic: str) -> None:
        self._pub.subscribe(listener, topic)

    def unsubscribe(self, listener: Callable[..., None], topic: str) -> None:
        try:
            self._pub.unsubscribe(listener, topic)
        except Exception:
            pass


#: (topic, handler method name). Signatures follow meshtastic-python's documented topics.
TOPICS = (
    ("meshtastic.node.updated", "on_node"),
    ("meshtastic.receive.position", "on_packet"),
    ("meshtastic.receive.user", "on_packet"),
    ("meshtastic.receive.telemetry", "on_packet"),
    ("meshtastic.connection.established", "on_established"),
    ("meshtastic.connection.lost", "on_lost"),
)


class _Interrupted(BaseException):
    """Raised from the signal handler to abort a blocking connect promptly."""


# --------------------------------------------------------------------------- the reader
class PresenceReader:
    """Connect, listen, snapshot, write. One instance runs one process lifetime."""

    def __init__(self, host: str, port: int, out_dir: str, *,
                 connector: Optional[Callable[[str, int, float], Any]] = None,
                 bus: Any = None, min_interval: float = 2.0, poll: float = 5.0,
                 connect_timeout: float = 60.0, backoff: Optional[Backoff] = None,
                 sleep: Optional[Callable[[float], None]] = None,
                 monotonic: Callable[[], float] = time.monotonic,
                 keepalive_interval: float = KEEPALIVE_INTERVAL) -> None:
        self.host, self.port, self.out_dir = host, port, out_dir
        self._connector = connector or default_connector
        self._bus = bus
        self.min_interval, self.poll, self.connect_timeout = min_interval, poll, connect_timeout
        #: never below 1 s: a misconfiguration must not turn the keep-alive into a flood
        self.keepalive_interval = max(1.0, float(keepalive_interval))
        self._backoff = backoff or Backoff()
        self._stop = threading.Event()
        self._sleep = sleep or (lambda seconds: self._stop.wait(seconds))
        self._monotonic = monotonic
        self._wake = threading.Event()
        self._dirty = threading.Event()
        self._lost = threading.Event()
        self._connecting = False
        self._iface: Any = None
        self._last_write: Optional[float] = None
        self._last_keepalive: Optional[float] = None
        self._last_bytes: Optional[bytes] = None
        self._subscribed: List[Tuple[Callable[..., None], str]] = []
        self.writes = 0
        self.keepalives = 0
        self.keepalive_failures = 0
        self.write_errors = 0
        self.dropped_nodes = 0
        self.connect_failures = 0

    # --- events (run on the library's publishing thread: set flags only) ---------------
    def _mark(self) -> None:
        self._dirty.set()
        self._wake.set()

    def on_node(self, node: Any = None, interface: Any = None) -> None:
        self._mark()

    def on_packet(self, packet: Any = None, interface: Any = None) -> None:
        self._mark()

    def on_established(self, interface: Any = None) -> None:
        self._mark()

    def on_lost(self, interface: Any = None) -> None:
        # a late event from an interface we already closed must not end the new session
        if interface is not None and self._iface is not None and interface is not self._iface:
            return
        self._lost.set()
        self._wake.set()

    def stop(self) -> None:
        self._stop.set()
        self._wake.set()

    def request_stop_from_signal(self) -> None:
        """Signal-handler entry: set the flags, abort a blocking connect."""
        self.stop()
        if self._connecting:
            raise _Interrupted()

    # --- subscription ------------------------------------------------------------------
    def _subscribe(self) -> None:
        if self._bus is None:
            self._bus = PubsubBus()
        for topic, name in TOPICS:
            listener = getattr(self, name)
            self._bus.subscribe(listener, topic)
            self._subscribed.append((listener, topic))

    def _unsubscribe(self) -> None:
        for listener, topic in self._subscribed:
            self._bus.unsubscribe(listener, topic)
        self._subscribed = []

    # --- snapshot ----------------------------------------------------------------------
    def snapshot(self, iface: Any) -> bytes:
        """The document bytes for the interface's current node DB."""
        own = _as_int(_attr(_attr(iface, "myInfo"), "my_node_num"))
        rows = normalize_nodes(_attr(iface, "nodesByNum"), skip_num=own)
        data, dropped = encode_document(rows)
        self.dropped_nodes = dropped
        return data

    def write_snapshot(self, iface: Any) -> bool:
        """Write the snapshot when it changed; True when a file was (re)written."""
        data = self.snapshot(iface)
        if data == self._last_bytes:
            return False
        try:
            write_atomic(self.out_dir, OUT_NAME, data)
        except OSError as exc:
            self.write_errors += 1
            LOG.warning("cannot write %s: %s", OUT_NAME, exc)
            return False
        self._last_bytes = data
        self.writes += 1
        LOG.info("wrote %s (%d bytes, %d dropped)", OUT_NAME, len(data), self.dropped_nodes)
        return True

    def _remove_stale(self) -> None:
        """A file from an earlier run must not make the service look LIVE; leftover
        temp files from a killed run are swept too."""
        try:
            os.unlink(os.path.join(self.out_dir, OUT_NAME))
        except FileNotFoundError:
            pass
        except OSError as exc:
            LOG.warning("cannot remove stale %s: %s", OUT_NAME, exc)
        try:
            for entry in os.listdir(self.out_dir):
                if entry.startswith("." + OUT_NAME + ".") and entry.endswith(".tmp"):
                    try:
                        os.unlink(os.path.join(self.out_dir, entry))
                    except OSError as exc:
                        LOG.warning("cannot remove stale temp file %s: %s", entry, exc)
        except OSError as exc:
            LOG.warning("cannot list %s for stale temp files: %s", self.out_dir, exc)
        self._last_bytes = None

    # --- one session -------------------------------------------------------------------
    @staticmethod
    def _iface_alive(iface: Any) -> bool:
        flag = _attr(iface, "isConnected")
        is_set = _attr(flag, "is_set")
        if callable(is_set):
            try:
                return bool(is_set())
            except Exception:
                return True
        return True

    def _keep_session_alive(self, iface: Any) -> None:
        """Send the session keep-alive when ``keepalive_interval`` has passed.

        The clock is the monotonic one and starts when the session's loop first runs, just
        after the connector returned (the ``want_config`` that opened the session restarted
        the board's idle timer then). A keep-alive that cannot be sent (socket error, a write
        that meshmud did not take within :data:`KEEPALIVE_WRITE_TIMEOUT`, no helper, refused
        by the guard) ends the session: the link is reported lost and the
        reader reconnects with back-off, whose fresh ``want_config`` revives a board session
        that has already lapsed.
        """
        now = self._monotonic()
        if self._last_keepalive is None:
            self._last_keepalive = now
            return
        if now - self._last_keepalive < self.keepalive_interval:
            return
        self._last_keepalive = now
        helper = _attr(iface, "session_keepalive")
        try:
            sent = bool(helper()) if callable(helper) else False
        except (socket.timeout, BlockingIOError) as exc:
            LOG.warning("session keep-alive write timed out (meshmud is not reading): %s", exc)
            sent = False
        except Exception as exc:
            LOG.warning("session keep-alive failed: %s", exc)
            sent = False
        if sent:
            self.keepalives += 1
            LOG.debug("session keep-alive sent")
            return
        self.keepalive_failures += 1
        LOG.warning("session keep-alive was not sent; treating the link as lost")
        self._lost.set()
        self._wake.set()

    def run_session(self, iface: Any, max_steps: Optional[int] = None) -> None:
        """Serve one connection until it is lost, the reader is stopped or ``max_steps``."""
        self._dirty.set()  # config is complete when the connector returns: publish now
        steps = 0
        while not self._stop.is_set() and not self._lost.is_set():
            if max_steps is not None and steps >= max_steps:
                return
            steps += 1
            self._keep_session_alive(iface)
            if self._lost.is_set():
                break
            if not self._dirty.is_set():
                self._wake.wait(self.poll)
                self._wake.clear()
                if self._stop.is_set() or self._lost.is_set():
                    break
                if not self._iface_alive(iface):
                    self._lost.set()
                    break
                if not self._dirty.is_set():
                    continue
            now = self._monotonic()
            if self._last_write is not None and now - self._last_write < self.min_interval:
                self._sleep(self.min_interval - (now - self._last_write))  # coalesce bursts
                if self._stop.is_set():
                    break
            self._dirty.clear()
            self.write_snapshot(iface)
            self._last_write = self._monotonic()

    # --- the loop ----------------------------------------------------------------------
    def _connect(self) -> Optional[Any]:
        self._iface = None
        self._last_write = None
        self._last_keepalive = None
        self._lost.clear()
        self._connecting = True
        try:
            iface = self._connector(self.host, self.port, self.connect_timeout)
            self._iface = iface
            return iface
        except _Interrupted:
            return None
        except Exception as exc:
            self.connect_failures += 1
            LOG.warning("connect to %s:%d failed: %s", self.host, self.port, exc)
            return None
        finally:
            self._connecting = False

    @staticmethod
    def _close(iface: Any) -> None:
        try:
            iface.close()
        except Exception as exc:
            LOG.debug("close: %s", exc)

    def run(self) -> int:
        """Run until stopped; returns the process exit code (0)."""
        self._subscribe()
        self._remove_stale()
        try:
            while not self._stop.is_set():
                iface = self._connect()
                if iface is None:
                    if not self._stop.is_set():
                        self._sleep(self._backoff.next())
                    continue
                self._backoff.reset()
                LOG.info("connected to %s:%d", self.host, self.port)
                try:
                    self.run_session(iface)
                finally:
                    self._close(iface)
                if not self._stop.is_set():
                    LOG.warning("link to %s:%d lost; reconnecting", self.host, self.port)
                    self._sleep(self._backoff.next())
        finally:
            self._unsubscribe()
        return 0


# --------------------------------------------------------------------------- CLI
def parse_endpoint(text: str) -> Tuple[str, int]:
    """``host:port`` (or ``[v6]:port``) to ``(host, port)``; raises ValueError."""
    host, sep, port_s = text.rpartition(":")
    if not sep or not host or not port_s.isdigit():
        raise ValueError("endpoint must be HOST:PORT")
    if host.startswith("[") and host.endswith("]"):
        host = host[1:-1]
    port = int(port_s)
    if not host or not 1 <= port <= 65535 or any(c in host for c in " \t\r\n/"):
        raise ValueError("endpoint must be HOST:PORT with a port in 1..65535")
    return host, port


def run_once(host: str, port: int, timeout: float,
             connector: Optional[Callable[[str, int, float], Any]] = None) -> Tuple[int, str]:
    """Identify mode: ``(exit_code, stdout_line)``; the line is empty on failure."""
    connect = connector or default_connector
    try:
        iface = connect(host, port, timeout)
    except Exception as exc:
        LOG.error("connect to %s:%d failed: %s", host, port, exc)
        return 3, ""
    try:
        info = build_my_node_info(iface)
    finally:
        PresenceReader._close(iface)
    if info is None:
        LOG.error("the node did not report my_node_info")
        return 3, ""
    return 0, json.dumps({"schema": SCHEMA, "my_node_info": info}, ensure_ascii=False,
                         separators=(",", ":"), allow_nan=False)


def build_parser() -> argparse.ArgumentParser:
    p = argparse.ArgumentParser(
        prog="meshtastic-presence",
        description="Read-only Meshtastic presence reader: writes <out>/meshtastic.json.")
    p.add_argument("--endpoint", default=DEFAULT_ENDPOINT, metavar="HOST:PORT",
                   help="Meshtastic TCP client API (meshmud downlink); default %(default)s")
    p.add_argument("--out", metavar="DIR", help="directory for meshtastic.json (must exist)")
    p.add_argument("--once", action="store_true",
                   help="identify: print my_node_info as JSON on stdout and exit; writes nothing")
    p.add_argument("--timeout", type=float, default=60.0, metavar="SECONDS",
                   help="how long to wait for the node database (default %(default)s)")
    p.add_argument("--min-interval", type=float, default=2.0, metavar="SECONDS",
                   help="minimum seconds between file writes (default %(default)s)")
    p.add_argument("-v", "--verbose", action="store_true", help="debug logging on stderr")
    p.add_argument("--version", action="version", version="meshtastic-presence " + __version__)
    return p


def main(argv: Optional[List[str]] = None) -> int:
    args = build_parser().parse_args(argv)
    logging.basicConfig(stream=sys.stderr, level=logging.DEBUG if args.verbose else logging.INFO,
                        format="meshtastic-presence: %(levelname)s %(message)s")
    try:
        host, port = parse_endpoint(args.endpoint)
    except ValueError as exc:
        LOG.error("%s", exc)
        return 2
    if args.once:
        code, line = run_once(host, port, args.timeout)
        if line:
            sys.stdout.write(line + "\n")
            sys.stdout.flush()
        return code
    if not args.out:
        LOG.error("--out DIR is required (or use --once)")
        return 2
    if not os.path.isdir(args.out):
        LOG.error("--out %s is not a directory", args.out)
        return 2
    reader = PresenceReader(host, port, args.out, connect_timeout=args.timeout,
                            min_interval=max(0.0, args.min_interval))

    def on_signal(signum: int, frame: Any) -> None:
        reader.request_stop_from_signal()

    signal.signal(signal.SIGTERM, on_signal)
    signal.signal(signal.SIGINT, on_signal)
    try:
        return reader.run()
    except _Interrupted:
        return 0


if __name__ == "__main__":
    sys.exit(main())
