#!/usr/bin/env python3
# lxmf-sidecar: the Layer 6 LXMF helper of the 111network OS (spec 60-transport/README.md section 6).
# Copyright (C) 2026 the 111network authors.
#
# Licence: PENDING the owner's contact with upstream (see LICENSE-NOTE.md). No licence is granted by
# this file yet; until it is settled the helper ships only behind the optional-component enable gate.
#
# Credit: Reticulum and LXMF are the work of the Reticulum project (reticulum.network). This program
# imports the user-installed, unmodified `rns` and `lxmf` packages; 111 uses them as released and is
# not affiliated with the Reticulum project. It is the one program of ours that imports Reticulum: a
# separate process in its own venv, never combined with `111d` (spec sections 6, 15, 18).
"""lxmf-sidecar: the LXMF helper (spec section 6).

A bidirectional JSON Lines bridge between ``111d`` and the node's shared ``rnsd`` instance. It attaches
to that instance as an interface-less client, owns one LXMF delivery identity and one destination, and
never listens on a port (spec section 15.7).

Protocol (UTF-8, one JSON object per line, at most 64 KiB, flushed per line):

* stdout is the protocol, stderr the log (never foreign text, never key material), stdin carries commands.
* ``111d -> helper``: ``{"type":"send","id","to","text","fields"?}`` | ``{"type":"announce"}`` |
  ``{"type":"name","display"}`` | ``{"type":"quit"}``. A malformed line is counted, answered by one
  stderr line and ignored; it never stops the helper.
* ``helper -> 111d``: ``ready`` (always the first line) | ``msg`` | ``state`` | ``heard`` | ``link``.

``fields`` of a ``send`` is a JSON object whose keys are decimal LXMF field ids (``"5"``) plus the one
name ``"relay"``: ``{"v","net","id","mid"}``, the relay marker (spec section 6, "Relay marker and
fields"), which the helper carries as ``FIELD_CUSTOM_TYPE = "111.relay"`` and ``FIELD_CUSTOM_DATA =
{v, net, id, mid}``. A received message is rendered the same way (``fields["relay"]`` appears only when
both custom fields are present and well formed).

Exit codes: 0 clean (``quit``, stdin closed, SIGTERM/SIGINT), 1 start-up failure, 2 usage, 3 partitioned
(the helper would have become the shared instance), 4 the ``rns``/``lxmf`` libraries are missing.

No wall clock orders or authenticates anything here: timers (announce interval, state sampling, link
check) use the monotonic clock; ``claim_ts`` is the sender's unverified claim, passed through for display.
"""
from __future__ import annotations

import argparse
import collections
import contextlib
import inspect
import json
import math
import os
import queue
import re
import signal
import sys
import threading
import time
import unicodedata
from typing import Any, Callable, Dict, List, Optional, Tuple

# --------------------------------------------------------------------------- limits (spec section 6)
MAX_LINE = 64 * 1024            # one protocol line, either direction (without the newline)
TEXT_CAP = 64 * 1024            # a received text is cut here (and again to fit its line)
ID_MAX = 128                    # a send id
NAME_MAX_BYTES = 128            # a display name, either direction
FIELD_BYTES_CAP = 256           # a bytes value in a rendered field is cut to this many bytes
FIELD_STR_CAP = 1024            # a string value in a rendered field
FIELD_ITEMS_CAP = 32            # entries per rendered container
FIELD_DEPTH_CAP = 5             # nesting depth of a rendered / accepted field value
FIELDS_JSON_CAP = 16 * 1024     # the rendered fields of one msg
MAX_SENDS = 1024                # tracked sends (concluded ones are evicted first)
MAX_PARKED = 256                # sends waiting for the peer's announce
MAX_NAMES = 2048                # announced display names remembered
MSGS_PER_SEND = 4               # library messages kept per send id (a retry after a post-air FAILED)
STATE_TICK_SECS = 1.0           # sampling of in-flight sends (only while one is in flight)
LINK_POLL_SECS = 5.0            # instance link check
DEFAULT_ANNOUNCE_MINUTES = 60
RELAY_TYPE = "111.relay"
INSTANCE_DEFAULT = "111"

# Fixed vocabulary of the protocol
S_QUEUED, S_SENT, S_RELAYED, S_DELIVERED, S_FAILED = "queued", "sent", "relayed", "delivered", "failed"
R_UNANNOUNCED = "peer_unannounced"
R_UNCONFIRMED = "transmitted, not confirmed"
R_REJECTED, R_CANCELLED = "rejected", "cancelled"

EXIT_OK, EXIT_START, EXIT_USAGE, EXIT_PARTITIONED, EXIT_NO_LIBS = 0, 1, 2, 3, 4

# LXMF field ids (LXMF.py); the library's own constants win when present.
_F_FILES, _F_IMAGE, _F_AUDIO, _F_TICKET = 0x05, 0x06, 0x07, 0x0C
_F_CUSTOM_TYPE, _F_CUSTOM_DATA = 0xFB, 0xFC
_DROPPED_FIELDS = (_F_FILES, _F_IMAGE, _F_AUDIO)  # attachments: replaced by {"attachment": {"bytes": N}}

_HEX32 = re.compile(r"^[0-9a-fA-F]{32}$")


# --------------------------------------------------------------------------- small helpers
class StartupError(Exception):
    """A start-up failure with the exit code to use; the message is our own fixed text."""

    def __init__(self, message: str, code: int = EXIT_START) -> None:
        Exception.__init__(self, message)
        self.code = code


def _ascii(text: Any, cap: int = 240) -> str:
    """Log-safe: ASCII only, no control characters, bounded. Used for every stderr line."""
    s = str(text).encode("ascii", "backslashreplace").decode("ascii")
    s = "".join(c if " " <= c < "\x7f" else "?" for c in s)
    return s[:cap]


def _clean_name(value: Any, limit: int = NAME_MAX_BYTES) -> Optional[str]:
    """A display name: no control characters or surrogates, cut to ``limit`` UTF-8 bytes on a code point."""
    if not isinstance(value, str):
        return None
    out = "".join(c for c in value if unicodedata.category(c) not in ("Cc", "Cs")).strip()
    raw = out.encode("utf-8")
    if len(raw) > limit:
        out = raw[:limit].decode("utf-8", "ignore").rstrip()
    return out or None


def _cut_utf8(raw: bytes, limit: int) -> Tuple[str, bool]:
    if len(raw) <= limit:
        return raw.decode("utf-8", "replace"), False
    return raw[:limit].decode("utf-8", "ignore"), True


def _hex(value: Any) -> Optional[str]:
    if isinstance(value, (bytes, bytearray)) and value:
        return bytes(value).hex()
    return None


class Log:
    """stderr: fixed text, counters, hex ids and exception class names. Never foreign text or keys."""

    def __init__(self, stream: Any, verbose: bool = False) -> None:
        self._stream = stream
        self.verbose = verbose
        self._lock = threading.Lock()

    def _w(self, level: str, text: str) -> None:
        line = "lxmf-sidecar: %s: %s\n" % (level, _ascii(text))
        try:
            with self._lock:
                self._stream.write(line)
                self._stream.flush()
        except Exception:
            pass

    def info(self, text: str) -> None:
        if self.verbose:
            self._w("info", text)

    def warn(self, text: str) -> None:
        self._w("warn", text)

    def error(self, text: str) -> None:
        self._w("error", text)


class Emitter:
    """The protocol writer: one JSON object per line, flushed, at most MAX_LINE bytes, thread safe."""

    def __init__(self, stream: Any, log: Log) -> None:
        self._stream = stream
        self._log = log
        self._lock = threading.Lock()
        self.closed = False
        self.refused = 0

    @staticmethod
    def encode(obj: Dict[str, Any]) -> bytes:
        return json.dumps(obj, ensure_ascii=False, separators=(",", ":"), allow_nan=False).encode("utf-8")

    def emit(self, obj: Dict[str, Any]) -> bool:
        try:
            raw = self.encode(obj)
        except (TypeError, ValueError):
            self.refused += 1
            self._log.warn("refused an unserialisable protocol line (type=%s)" % _ascii(obj.get("type")))
            return False
        if len(raw) > MAX_LINE:
            self.refused += 1
            self._log.warn("refused an oversize protocol line (type=%s, %d bytes)" % (_ascii(obj.get("type")), len(raw)))
            return False
        with self._lock:
            if self.closed:
                return False
            try:
                self._stream.write(raw + b"\n")
                self._stream.flush()
            except (OSError, ValueError):
                self.closed = True  # 111d is gone: the loop ends on the next wake
                return False
        return True


# --------------------------------------------------------------------------- commands (111d -> helper)
class BadCommand(Exception):
    """A malformed command line; ``reason`` is a fixed token (never the line's text)."""

    def __init__(self, reason: str) -> None:
        Exception.__init__(self, reason)
        self.reason = reason


def _json_value(value: Any, depth: int = 0) -> Any:
    """A JSON value accepted as an LXMF field value: str, int, float, bool, null, list, object."""
    if depth > FIELD_DEPTH_CAP:
        raise BadCommand("fields_depth")
    if value is None or isinstance(value, (bool, str)):
        return value
    if isinstance(value, int):
        return value
    if isinstance(value, float):
        if not math.isfinite(value):
            raise BadCommand("fields_number")
        return value
    if isinstance(value, list):
        return [_json_value(v, depth + 1) for v in value]
    if isinstance(value, dict):
        return {str(k): _json_value(v, depth + 1) for k, v in value.items()}
    raise BadCommand("fields_type")


def _relay_marker(obj: Any) -> Dict[str, Any]:
    """Validate ``{v, net, id, mid}`` (spec section 6 relay marker)."""
    if not isinstance(obj, dict):
        raise BadCommand("relay_marker")
    v, net, rid, mid = obj.get("v"), obj.get("net"), obj.get("id"), obj.get("mid")
    if isinstance(v, bool) or not isinstance(v, int) or not 0 <= v <= 255:
        raise BadCommand("relay_marker")
    for s, cap in ((net, 64), (rid, 256), (mid, 256)):
        if not isinstance(s, str) or not s or len(s.encode("utf-8")) > cap:
            raise BadCommand("relay_marker")
    return {"v": v, "net": net, "id": rid, "mid": mid}


def parse_command(raw: bytes) -> Dict[str, Any]:
    """Validate one stdin line. Raises BadCommand with a fixed reason token."""
    if len(raw) > MAX_LINE:
        raise BadCommand("oversize")
    try:
        text = raw.decode("utf-8")
        obj = json.loads(text)
    except (UnicodeDecodeError, ValueError, RecursionError):
        raise BadCommand("bad_json")
    if not isinstance(obj, dict):
        raise BadCommand("not_object")
    kind = obj.get("type")
    if kind in ("announce", "quit"):
        return {"type": kind}
    if kind == "name":
        display = _clean_name(obj.get("display"))
        if display is None:
            raise BadCommand("name")
        return {"type": "name", "display": display}
    if kind == "send":
        sid, to, body = obj.get("id"), obj.get("to"), obj.get("text")
        if not isinstance(sid, str) or not 1 <= len(sid) <= ID_MAX or any(unicodedata.category(c) == "Cc" for c in sid):
            raise BadCommand("send_id")
        if not isinstance(to, str) or not _HEX32.match(to):
            raise BadCommand("send_to")
        if not isinstance(body, str):
            raise BadCommand("send_text")
        fields = obj.get("fields")
        if fields is not None and not isinstance(fields, dict):
            raise BadCommand("send_fields")
        return {"type": "send", "id": sid, "to": bytes.fromhex(to), "text": body, "fields": fields or {}}
    raise BadCommand("unknown_type")


def fields_from_json(fields: Dict[str, Any], custom_type: int = _F_CUSTOM_TYPE,
                     custom_data: int = _F_CUSTOM_DATA) -> Dict[int, Any]:
    """The LXMF ``fields`` dict of a send: decimal-id keys as given, ``relay`` as the custom fields."""
    out: Dict[int, Any] = {}
    marker: Optional[Dict[str, Any]] = None
    for key, value in fields.items():
        if key == "relay":
            marker = _relay_marker(value)
        elif isinstance(key, str) and key.isdigit() and len(key) <= 3 and int(key) <= 255:
            out[int(key)] = _json_value(value)
        else:
            raise BadCommand("fields_key")
    if marker is not None:
        out[custom_type] = RELAY_TYPE
        out[custom_data] = marker
    return out


# --------------------------------------------------------------------------- received fields (helper -> 111d)
def _count_bytes(value: Any, depth: int = 0) -> int:
    if isinstance(value, (bytes, bytearray)):
        return len(value)
    if depth >= FIELD_DEPTH_CAP:
        return 0
    if isinstance(value, (list, tuple)):
        return sum(_count_bytes(v, depth + 1) for v in list(value)[:1024])
    if isinstance(value, dict):
        return sum(_count_bytes(v, depth + 1) for v in list(value.values())[:1024])
    return 0


def _render_value(value: Any, flags: Dict[str, bool], depth: int = 0) -> Any:
    if value is None or isinstance(value, bool):
        return value
    if isinstance(value, int):
        return value if abs(value) < 2 ** 53 else str(value)
    if isinstance(value, float):
        return value if math.isfinite(value) else None
    if isinstance(value, (bytes, bytearray)):
        if len(value) > FIELD_BYTES_CAP:
            flags["fields_truncated"] = True
        return bytes(value[:FIELD_BYTES_CAP]).hex()
    if isinstance(value, str):
        if len(value) > FIELD_STR_CAP:
            flags["fields_truncated"] = True
        return "".join(c for c in value[:FIELD_STR_CAP] if unicodedata.category(c) not in ("Cc", "Cs"))
    if depth >= FIELD_DEPTH_CAP:
        flags["fields_truncated"] = True
        return None
    if isinstance(value, (list, tuple)):
        items = list(value)
        if len(items) > FIELD_ITEMS_CAP:
            flags["fields_truncated"] = True
        return [_render_value(v, flags, depth + 1) for v in items[:FIELD_ITEMS_CAP]]
    if isinstance(value, dict):
        out: Dict[str, Any] = {}
        for k, v in list(value.items())[:FIELD_ITEMS_CAP]:
            if isinstance(k, (str, int)) and not isinstance(k, bool):
                out[_render_value(k, flags, depth + 1) if isinstance(k, str) else str(k)] = _render_value(v, flags, depth + 1)
        if len(value) > FIELD_ITEMS_CAP:
            flags["fields_truncated"] = True
        return out
    flags["fields_truncated"] = True
    return None


def render_fields(fields: Any, custom_type: int = _F_CUSTOM_TYPE,
                  custom_data: int = _F_CUSTOM_DATA) -> Tuple[Dict[str, Any], bool]:
    """A received fields dict as a JSON object: int keys as decimal strings, bytes as capped hex,
    attachments replaced by ``{"attachment": {"bytes": N}}``, tickets dropped, everything bounded.
    Returns ``(rendered, truncated)``."""
    flags: Dict[str, bool] = {}
    out: Dict[str, Any] = {}
    if not isinstance(fields, dict):
        return out, False
    for key, value in list(fields.items())[:FIELD_ITEMS_CAP * 2]:
        if not isinstance(key, int) or isinstance(key, bool):
            flags["fields_truncated"] = True  # only integer ids exist in LXMF; anything else is noise
            continue
        if key in _DROPPED_FIELDS:
            out[str(key)] = {"attachment": {"bytes": _count_bytes(value)}}
        elif key == _F_TICKET:
            continue
        else:
            out[str(key)] = _render_value(value, flags)
    # The relay marker, only when both custom fields are present and well formed.
    ctype, cdata = fields.get(custom_type), fields.get(custom_data)
    if isinstance(ctype, (bytes, bytearray)):
        ctype = bytes(ctype).decode("utf-8", "replace")
    if ctype == RELAY_TYPE and isinstance(cdata, dict):
        try:
            clean = {(k.decode("utf-8", "replace") if isinstance(k, (bytes, bytearray)) else k):
                     (v.decode("utf-8", "replace") if isinstance(v, (bytes, bytearray)) else v)
                     for k, v in cdata.items() if isinstance(k, (str, bytes, bytearray))}
            out["relay"] = _relay_marker(clean)
        except BadCommand:
            pass
    # Whole-object bound: drop the largest entries until it fits.
    while len(json.dumps(out, ensure_ascii=False, separators=(",", ":"))) > FIELDS_JSON_CAP and out:
        biggest = max(out, key=lambda k: len(json.dumps(out[k], ensure_ascii=False)))
        del out[biggest]
        flags["fields_truncated"] = True
    return out, bool(flags)


# --------------------------------------------------------------------------- send states (spec section 6, v0.3 item 1)
_RANK = {S_QUEUED: 1, S_SENT: 2, S_RELAYED: 3, S_DELIVERED: 4, S_FAILED: 5}


class LibStates:
    """The ``LXMessage`` state constants, read from the library (hard-coded fall-backs are LXMF's)."""

    def __init__(self, lxmessage: Any) -> None:
        g = lambda name, default: getattr(lxmessage, name, default)  # noqa: E731
        self.GENERATING = g("GENERATING", 0x00)
        self.OUTBOUND = g("OUTBOUND", 0x01)
        self.SENDING = g("SENDING", 0x02)
        self.SENT = g("SENT", 0x04)
        self.DELIVERED = g("DELIVERED", 0x08)
        self.REJECTED = g("REJECTED", 0xFD)
        self.CANCELLED = g("CANCELLED", 0xFE)
        self.FAILED = g("FAILED", 0xFF)
        self.PROPAGATED = g("PROPAGATED", 0x03)
        self.OPPORTUNISTIC = g("OPPORTUNISTIC", 0x01)


def map_library_state(state: Any, propagated: bool, air: bool, c: LibStates) -> Optional[Tuple[str, Optional[str]]]:
    """The library state of one message as ``(protocol state, reason)``; ``air`` = it was seen SENDING/SENT.

    OUTBOUND queued · SENDING/SENT sent · propagated SENT relayed · DELIVERED delivered · FAILED after air
    sent "transmitted, not confirmed" · FAILED before air queued peer_unannounced · REJECTED and
    CANCELLED failed."""
    if state in (c.GENERATING, c.OUTBOUND):
        return S_QUEUED, None
    if state == c.SENDING:
        return S_SENT, None
    if state == c.SENT:
        return (S_RELAYED, None) if propagated else (S_SENT, None)
    if state == c.DELIVERED:
        return S_DELIVERED, None
    if state == c.FAILED:
        return (S_SENT, R_UNCONFIRMED) if air else (S_QUEUED, R_UNANNOUNCED)
    if state == c.REJECTED:
        return S_FAILED, R_REJECTED
    if state == c.CANCELLED:
        return S_FAILED, R_CANCELLED
    return None


def may_emit(prior: Optional[str], prior_reason: Optional[str], state: str, reason: Optional[str]) -> bool:
    """The monotone rule: delivered and failed are final; a state only moves up; a failed needs a
    queued/sent row; an equal queued/sent row may only add a new reason."""
    if prior is None:
        return True
    if prior in (S_DELIVERED, S_FAILED):
        return False
    if state == S_FAILED:
        return prior in (S_QUEUED, S_SENT)
    if _RANK[state] > _RANK[prior]:
        return True
    return state == prior and state in (S_QUEUED, S_SENT) and reason is not None and reason != prior_reason


class SendEntry:
    __slots__ = ("id", "dest", "text", "fields", "msgs", "state", "reason", "air", "parked", "concluded")

    def __init__(self, sid: str, dest: bytes, text: str, fields: Dict[int, Any]) -> None:
        self.id = sid
        self.dest = dest
        self.text = text
        self.fields = fields
        self.msgs: List[Any] = []
        self.state: Optional[str] = None
        self.reason: Optional[str] = None
        self.air = False
        self.parked = False
        self.concluded = False  # the library is done with the current message (delivered, failed, relayed)


# --------------------------------------------------------------------------- client config (spec sections 6 (1), 11)
def _section_of(line: str) -> Optional[str]:
    s = line.strip()
    if s.startswith("[[") and s.endswith("]]"):
        return "[[" + s[2:-2].strip().lower() + "]]"
    if s.startswith("[") and s.endswith("]"):
        return s[1:-1].strip().lower()
    return None


def ensure_client_config(config_dir: str, log: Log) -> str:
    """Make ``<config_dir>/config`` an interface-less shared-instance client config and return the
    instance name. ``111d`` renders this file (including ``rpc_key``); the helper only refreshes
    ``share_instance`` / ``instance_name`` when they drifted and never writes or changes an ``rpc_key``.
    Dir 0700, file 0600. A config that declares interfaces is refused: the helper opens no other path."""
    os.makedirs(config_dir, mode=0o700, exist_ok=True)
    try:
        os.chmod(config_dir, 0o700)
    except OSError:
        pass
    path = os.path.join(config_dir, "config")
    instance = INSTANCE_DEFAULT
    if not os.path.exists(path):
        log.warn("no client config in the config dir; wrote one without an rpc_key (111d renders it)")
        body = ("[reticulum]\n  share_instance = Yes\n  instance_name = %s\n\n[interfaces]\n" % instance)
        _write_private(path, body)
        return instance
    if os.path.getsize(path) > MAX_LINE:
        raise StartupError("the client config is larger than 64 KiB", EXIT_USAGE)
    with open(path, "r", encoding="utf-8", errors="replace") as f:
        lines = f.read().splitlines()
    section = ""
    seen_share = seen_instance = False
    out: List[str] = []
    changed = False
    reticulum_at = -1
    for line in lines:
        sec = _section_of(line)
        if sec is not None:
            section = sec
            if sec.startswith("[["):
                raise StartupError("the client config declares an interface; the helper is interface-less", EXIT_USAGE)
            if sec == "reticulum":
                reticulum_at = len(out)
            out.append(line)
            continue
        if section == "reticulum" and "=" in line and not line.strip().startswith("#"):
            key, _, value = line.partition("=")
            key, value = key.strip().lower(), value.strip()
            if key == "share_instance":
                seen_share = True
                if value.lower() not in ("yes", "true", "1", "on"):
                    out.append("  share_instance = Yes")
                    changed = True
                    continue
            elif key == "instance_name":
                seen_instance = True
                if value:
                    instance = value
            # an rpc_key line is kept byte for byte
        out.append(line)
    add: List[str] = []
    if not seen_share:
        add.append("  share_instance = Yes")
    if not seen_instance:
        add.append("  instance_name = %s" % instance)
    if add:
        changed = True
        if reticulum_at >= 0:
            out[reticulum_at + 1:reticulum_at + 1] = add
        else:
            out = ["[reticulum]"] + add + [""] + out
    mode = os.stat(path).st_mode & 0o777
    if changed:
        _write_private(path, "\n".join(out) + "\n")
    elif mode != 0o600:
        os.chmod(path, 0o600)
    return instance


def _write_private(path: str, text: str) -> None:
    tmp = path + ".tmp"
    fd = os.open(tmp, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
    try:
        with os.fdopen(fd, "w", encoding="utf-8") as f:
            f.write(text)
            f.flush()
            os.fsync(f.fileno())
    except BaseException:
        try:
            os.unlink(tmp)
        except OSError:
            pass
        raise
    os.chmod(tmp, 0o600)
    os.replace(tmp, path)


def load_identity(RNS: Any, path: str) -> Any:
    """Load ``--identity`` or create it (0600). A file that exists but cannot be read is never replaced."""
    parent = os.path.dirname(os.path.abspath(path))
    os.makedirs(parent, mode=0o700, exist_ok=True)
    if os.path.exists(path):
        ident = RNS.Identity.from_file(path)
        if ident is None:
            raise StartupError("the identity file exists but cannot be read; not replacing it")
        try:
            os.chmod(path, 0o600)
        except OSError:
            pass
        return ident
    ident = RNS.Identity()
    fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    os.close(fd)
    try:
        ident.to_file(path)
    except BaseException:
        try:
            os.unlink(path)
        except OSError:
            pass
        raise
    return ident


# --------------------------------------------------------------------------- the helper
class _AnnounceHandler:
    """RNS announce handler for the ``lxmf.delivery`` aspect (three-argument form)."""

    def __init__(self, helper: "Helper") -> None:
        self.aspect_filter = "lxmf.delivery"
        self.receive_path_responses = True
        self._helper = helper

    def received_announce(self, destination_hash, announced_identity, app_data):  # noqa: ANN001
        self._helper.on_announce(destination_hash, app_data)


class Helper:
    """All state of one run. Library callbacks arrive on library threads; one RLock guards the state."""

    def __init__(self, args: Any, RNS: Any, LXMF: Any, out: Emitter, log: Log,
                 clock: Callable[[], float] = time.monotonic) -> None:
        self.args = args
        self.RNS = RNS
        self.LXMF = LXMF
        self.out = out
        self.log = log
        self.clock = clock
        self.lib = LibStates(getattr(LXMF, "LXMessage"))
        self.f_custom_type = getattr(LXMF, "FIELD_CUSTOM_TYPE", _F_CUSTOM_TYPE)
        self.f_custom_data = getattr(LXMF, "FIELD_CUSTOM_DATA", _F_CUSTOM_DATA)
        self.lock = threading.RLock()
        self.sends: "collections.OrderedDict[str, SendEntry]" = collections.OrderedDict()
        self.names: "collections.OrderedDict[bytes, str]" = collections.OrderedDict()
        self.stop = threading.Event()
        self.malformed = 0
        self.router: Any = None
        self.dest: Any = None
        self.identity: Any = None
        self.reticulum: Any = None
        self.instance = INSTANCE_DEFAULT
        self.link_state: Optional[str] = None
        self.next_announce: Optional[float] = None
        self.next_link_poll = 0.0
        self.next_state_tick = 0.0

    # ---- attach / start -------------------------------------------------------------------------
    def attach(self) -> None:
        """Spec section 6 (1)-(2): client config, shared-instance attach, identity, router, destination.
        Raises StartupError (code 3 = partitioned)."""
        RNS = self.RNS
        self.instance = ensure_client_config(self.args.config_dir, self.log)
        kwargs: Dict[str, Any] = {"configdir": self.args.config_dir,
                                  "loglevel": getattr(RNS, "LOG_INFO", 4) if self.args.verbose else getattr(RNS, "LOG_WARNING", 2),
                                  "logdest": self._rns_log}
        try:
            if "require_shared_instance" in inspect.signature(RNS.Reticulum.__init__).parameters:
                kwargs["require_shared_instance"] = True  # RNS >= 1.5.7: never become the instance
        except (TypeError, ValueError):
            pass
        try:
            self.reticulum = RNS.Reticulum(**kwargs)
        except SystemError:
            raise StartupError("a shared instance is required but none is serving", EXIT_PARTITIONED)
        except Exception as exc:  # the library's own failure; the class name is ours, the text may not be
            raise StartupError("Reticulum failed to start (%s)" % type(exc).__name__)
        if not getattr(self.reticulum, "is_connected_to_shared_instance", False):
            # No shared instance was found: this process became (or would be) the instance. Never go on.
            raise StartupError("not attached to a shared instance", EXIT_PARTITIONED)
        self.identity = load_identity(RNS, self.args.identity)
        os.makedirs(self.args.storage, mode=0o700, exist_ok=True)
        try:
            os.chmod(self.args.storage, 0o700)
        except OSError:
            pass
        self.router = self.LXMF.LXMRouter(identity=self.identity, storagepath=self.args.storage)
        name = _clean_name(self.args.name, 64)
        self.dest = self.router.register_delivery_identity(self.identity, display_name=name or INSTANCE_DEFAULT,
                                                           stamp_cost=None)  # no stamp cost is advertised
        if self.dest is None:
            raise StartupError("the delivery destination could not be registered")
        if name is None:
            self.dest.display_name = "111-" + bytes(self.dest.hash).hex()[:4]
        self.router.register_delivery_callback(self.on_inbound)
        RNS.Transport.register_announce_handler(_AnnounceHandler(self))

    def ready_line(self) -> Dict[str, Any]:
        pub = self.identity.get_public_key()
        return {"type": "ready", "dest": bytes(self.dest.hash).hex(), "pubkey": bytes(pub).hex(), "instance": self.instance}

    def start(self) -> None:
        """Emit ``ready`` (always the first line), the link state and the start announce."""
        self.out.emit(self.ready_line())
        now = self.clock()
        self.link_state = self._link_now()
        self.out.emit({"type": "link", "state": self.link_state})
        self.next_link_poll = now + LINK_POLL_SECS
        self.do_announce("start")

    # ---- logging from the library ---------------------------------------------------------------
    def _rns_log(self, text: Any) -> None:
        self.log._w("rns", _ascii(text, 200))

    # ---- announces ------------------------------------------------------------------------------
    def do_announce(self, why: str) -> None:
        try:
            self.router.announce(self.dest.hash)
            self.log.info("announced (%s)" % why)
        except Exception as exc:
            self.log.warn("announce failed (%s)" % type(exc).__name__)
        minutes = self.args.announce_interval
        self.next_announce = self.clock() + minutes * 60.0 if minutes and minutes > 0 else None

    def on_announce(self, dest_hash: Any, app_data: Any) -> None:
        """An ``lxmf.delivery`` announce was heard: report it and resume sends parked on it."""
        try:
            d = bytes(dest_hash)
            if self.dest is not None and d == bytes(self.dest.hash):
                return
            name = self._announced_name(app_data)
            if name is not None:
                with self.lock:
                    self.names[d] = name
                    self.names.move_to_end(d)
                    while len(self.names) > MAX_NAMES:
                        self.names.popitem(last=False)
            self.out.emit({"type": "heard", "dest": d.hex(), "name": name})
            ready: List[Tuple[SendEntry, Any]] = []
            with self.lock:
                for e in [e for e in self.sends.values() if e.parked and e.dest == d]:
                    m = self._prepare(e)
                    if m is not None:
                        ready.append((e, m))
            for e, m in ready:
                self._handoff(e, m)  # outside the lock: the router may wait on its own lock
        except Exception as exc:
            self.log.warn("announce handling failed (%s)" % type(exc).__name__)

    def _announced_name(self, app_data: Any) -> Optional[str]:
        if not app_data:
            return None
        try:
            return _clean_name(self.LXMF.display_name_from_app_data(app_data))
        except Exception:
            return None

    def _known_name(self, src: bytes) -> Optional[str]:
        with self.lock:
            if src in self.names:
                return self.names[src]
        try:
            return self._announced_name(self.RNS.Identity.recall_app_data(src))
        except Exception:
            return None

    # ---- commands -------------------------------------------------------------------------------
    def handle_line(self, raw: bytes) -> bool:
        """One stdin line. Returns False when the helper should stop (``quit``)."""
        try:
            cmd = parse_command(raw.rstrip(b"\r\n"))
        except BadCommand as bad:
            self.malformed += 1
            self.log.warn("ignored a malformed command (reason=%s, total=%d)" % (bad.reason, self.malformed))
            return True
        kind = cmd["type"]
        if kind == "quit":
            return False
        try:
            if kind == "announce":
                self.do_announce("command")
            elif kind == "name":
                self.dest.display_name = cmd["display"]
                self.do_announce("name")
            elif kind == "send":
                self.cmd_send(cmd)
        except Exception as exc:  # never crash on a command
            self.log.error("command %s failed (%s)" % (kind, type(exc).__name__))
        return True

    def cmd_send(self, cmd: Dict[str, Any]) -> None:
        try:
            fields = fields_from_json(cmd["fields"], self.f_custom_type, self.f_custom_data)
        except BadCommand as bad:
            self.malformed += 1
            self.log.warn("ignored a send with bad fields (reason=%s, total=%d)" % (bad.reason, self.malformed))
            return
        m = None
        with self.lock:
            e = self.sends.get(cmd["id"])
            if e is not None:
                if e.concluded and e.state in (S_QUEUED, S_SENT) and not e.parked:
                    # 111d re-queues after a post-air FAILED: a new attempt under the same id; the
                    # emitted states stay monotone across attempts.
                    e.text, e.fields, e.dest, e.concluded, e.air = cmd["text"], fields, cmd["to"], False, False
                    m = self._prepare(e)
                else:
                    self.log.warn("ignored a send whose id is already in flight or final")
                    return
            elif not self._make_room():
                self.out.emit({"type": "state", "id": cmd["id"], "state": S_FAILED, "reason": "helper_busy"})
                return
            else:
                e = SendEntry(cmd["id"], cmd["to"], cmd["text"], fields)
                self.sends[e.id] = e
                m = self._prepare(e)
        if m is not None:
            self._handoff(e, m)

    def _make_room(self) -> bool:
        if len(self.sends) < MAX_SENDS:
            return True
        for sid, e in list(self.sends.items()):
            if e.concluded and not e.parked:
                del self.sends[sid]
                return True
        return False

    # ---- sending --------------------------------------------------------------------------------
    def _prepare(self, e: SendEntry) -> Optional[Any]:
        """Caller holds the lock. Build the library message for ``e`` and register it, or park the send
        until the peer announces (None). Never calls the router."""
        RNS, LXMF = self.RNS, self.LXMF
        ident = RNS.Identity.recall(e.dest)
        if ident is None:
            if not e.parked and sum(1 for x in self.sends.values() if x.parked) >= MAX_PARKED:
                self._advance(e, S_FAILED, "helper_busy")
                e.concluded = True
                return None
            e.parked = True
            self._advance(e, S_QUEUED, R_UNANNOUNCED)
            return None
        e.parked = False
        try:
            d = RNS.Destination(ident, RNS.Destination.OUT, RNS.Destination.SINGLE, "lxmf", "delivery")
            if bytes(d.hash) != e.dest:
                raise ValueError("destination mismatch")
            # The library's own pick: opportunistic when the message fits one packet, else a link.
            m = LXMF.LXMessage(d, self.dest, e.text, fields=e.fields or None, desired_method=self.lib.OPPORTUNISTIC)
            sid = e.id
            m.register_delivery_callback(lambda msg, sid=sid: self._on_outbound(sid, msg))
            m.register_failed_callback(lambda msg, sid=sid: self._on_outbound(sid, msg))
        except Exception as exc:
            self.log.warn("send could not be built (%s)" % type(exc).__name__)
            self._advance(e, S_FAILED, "send_error")
            e.concluded = True
            return None
        e.msgs.append(m)
        del e.msgs[:-MSGS_PER_SEND]
        return m

    def _handoff(self, e: SendEntry, m: Any) -> None:
        """Hand one prepared message to the router. The caller must NOT hold the lock: ``handle_outbound``
        waits for the router's outbound lock, whose holder may be calling our failed callback."""
        try:
            self.router.handle_outbound(m)  # never auto-resend: one hand-over per command
        except Exception as exc:
            self.log.warn("send could not be handed to the router (%s)" % type(exc).__name__)
            with self.lock:
                self._advance(e, S_FAILED, "send_error")
                e.concluded = True
            return
        with self.lock:
            self._observe(e, m, stale=bool(e.msgs) and m is not e.msgs[-1])

    def _on_outbound(self, sid: str, m: Any) -> None:
        try:
            with self.lock:
                e = self.sends.get(sid)
                if e is None or not any(m is x for x in e.msgs):
                    return
                self._observe(e, m, stale=m is not e.msgs[-1])
        except Exception as exc:
            self.log.warn("state callback failed (%s)" % type(exc).__name__)

    def _observe(self, e: SendEntry, m: Any, stale: bool = False) -> None:
        """Read one library message's state and advance the send (monotone). Caller holds the lock."""
        st = getattr(m, "state", None)
        if stale and st != self.lib.DELIVERED:
            return  # an older attempt may still prove delivery; nothing else of it counts
        if st in (self.lib.SENDING, self.lib.SENT):
            e.air = True
        propagated = getattr(m, "method", None) == self.lib.PROPAGATED
        mapped = map_library_state(st, propagated, e.air, self.lib)
        if mapped is None:
            return
        if st in (self.lib.DELIVERED, self.lib.FAILED, self.lib.REJECTED, self.lib.CANCELLED) or mapped[0] == S_RELAYED:
            e.concluded = True
        self._advance(e, mapped[0], mapped[1], m)

    def _advance(self, e: SendEntry, state: str, reason: Optional[str], m: Any = None) -> None:
        if not may_emit(e.state, e.reason, state, reason):
            return
        e.state, e.reason = state, reason
        line: Dict[str, Any] = {"type": "state", "id": e.id}
        h = _hex(getattr(m if m is not None else (e.msgs[-1] if e.msgs else None), "hash", None))
        if h:
            line["hash"] = h
        line["state"] = state
        if reason:
            line["reason"] = reason
        self.out.emit(line)

    # ---- receiving ------------------------------------------------------------------------------
    def on_inbound(self, message: Any) -> None:
        """The router's delivery callback (spec section 6 (5)). Attachments are never forwarded."""
        try:
            h, src = _hex(getattr(message, "hash", None)), _hex(getattr(message, "source_hash", None))
            if h is None or src is None:
                return
            content = getattr(message, "content", b"")
            if isinstance(content, str):
                content = content.encode("utf-8", "replace")
            text, truncated = _cut_utf8(bytes(content or b""), TEXT_CAP)
            fields, ftrunc = render_fields(getattr(message, "fields", None), self.f_custom_type, self.f_custom_data)
            ts = getattr(message, "timestamp", None)
            claim = float(ts) if isinstance(ts, (int, float)) and not isinstance(ts, bool) and math.isfinite(ts) else None
            signed = bool(getattr(message, "signature_validated", False))
            why: Optional[str] = None
            if not signed:
                reason = getattr(message, "unverified_reason", None)
                lm = getattr(self.LXMF, "LXMessage", None)
                if reason is not None and reason == getattr(lm, "SOURCE_UNKNOWN", 0x01):
                    why = "source_unknown"
                elif reason is not None and reason == getattr(lm, "SIGNATURE_INVALID", 0x02):
                    why = "signature_invalid"
                else:
                    why = "unverified"
            line: Dict[str, Any] = {"type": "msg", "hash": h, "src": src, "name": self._known_name(bytes.fromhex(src)),
                                    "text": text, "claim_ts": claim, "signed": signed, "why_unsigned": why,
                                    "fields": fields}
            if truncated:
                line["truncated"] = True
            if ftrunc:
                line["fields_truncated"] = True
            # The whole line must stay within MAX_LINE: shrink the text, never the envelope.
            while len(Emitter.encode(line)) > MAX_LINE and line["text"]:
                over = len(Emitter.encode(line)) - MAX_LINE
                keep = max(0, len(line["text"]) - max(over // 6 + 1, len(line["text"]) // 8))
                line["text"] = line["text"][:keep]
                line["truncated"] = True
            self.out.emit(line)
        except Exception as exc:
            self.log.warn("inbound message dropped (%s)" % type(exc).__name__)

    # ---- timers ---------------------------------------------------------------------------------
    def _link_now(self) -> str:
        try:
            for i in list(getattr(self.RNS.Transport, "interfaces", None) or []):
                if type(i).__name__ == "LocalClientInterface":
                    return "up" if getattr(i, "online", False) else "down"
        except Exception:
            pass
        return "down"

    def _pending(self) -> bool:
        return any((not e.concluded) and (not e.parked) and e.msgs for e in self.sends.values())

    def next_wakeup(self) -> float:
        """Seconds until the next timer. The state sample is scheduled only while a send is in flight; the
        link check and the announce interval are the helper's only other timers."""
        now = self.clock()
        due = [self.next_link_poll]
        if self.next_announce is not None:
            due.append(self.next_announce)
        with self.lock:
            if self._pending():
                due.append(self.next_state_tick)
        return max(0.0, min(due) - now)

    def tick(self) -> None:
        """Run whatever timer is due (monotonic clock)."""
        now = self.clock()
        if now >= self.next_link_poll:
            self.next_link_poll = now + LINK_POLL_SECS
            state = self._link_now()
            if state != self.link_state:
                self.link_state = state
                self.out.emit({"type": "link", "state": state})
                if state == "up":
                    self.do_announce("link up")
        if self.next_announce is not None and now >= self.next_announce:
            self.do_announce("interval")
        if now >= self.next_state_tick:
            self.next_state_tick = now + STATE_TICK_SECS
            with self.lock:
                for e in list(self.sends.values()):
                    if not e.concluded and not e.parked and e.msgs:
                        self._observe(e, e.msgs[-1])

    # ---- main loop ------------------------------------------------------------------------------
    def run(self, inbox: "queue.Queue[Tuple[str, Any]]") -> None:
        while not self.stop.is_set() and not self.out.closed:
            timeout = self.next_wakeup()
            try:
                kind, payload = inbox.get(timeout=timeout)
            except queue.Empty:
                kind, payload = "tick", None
            if kind == "eof" or kind == "quit":
                break
            if kind == "line":
                if not self.handle_line(payload):
                    break
            elif kind == "bad":
                self.malformed += 1
                self.log.warn("ignored a malformed command (reason=%s, total=%d)" % (payload, self.malformed))
            self.tick()

    def shutdown(self) -> None:
        for fn in (getattr(self.router, "exit_handler", None), getattr(getattr(self.RNS, "Reticulum", None), "exit_handler", None)):
            if callable(fn):
                try:
                    fn()
                except Exception:
                    pass


# --------------------------------------------------------------------------- stdin reader
def read_commands(stream: Any, put: Callable[[Tuple[str, Any]], None]) -> None:
    """Read newline-framed commands from a binary stream; an oversize line is drained and reported."""
    while True:
        try:
            raw = stream.readline(MAX_LINE + 1)
        except (OSError, ValueError):
            put(("eof", None))
            return
        if not raw:
            put(("eof", None))
            return
        if len(raw) > MAX_LINE and not raw.endswith(b"\n"):
            while True:
                try:
                    chunk = stream.readline(MAX_LINE + 1)
                except (OSError, ValueError):
                    chunk = b""
                if not chunk or chunk.endswith(b"\n"):
                    break
            put(("bad", "oversize"))
            continue
        if raw.strip():
            put(("line", raw))


# --------------------------------------------------------------------------- entry point
def build_parser() -> argparse.ArgumentParser:
    ap = argparse.ArgumentParser(prog="lxmf_sidecar.py", description=__doc__.split("\n\n")[0])
    ap.add_argument("--config-dir", required=True, help="the Reticulum client config dir (bridge/lxmf/rns)")
    ap.add_argument("--identity", required=True, help="the delivery identity file (created 0600 when absent)")
    ap.add_argument("--storage", required=True, help="the LXMF router storage dir")
    ap.add_argument("--name", default=None, help="display name (default 111-<first 4 hex of the address>)")
    ap.add_argument("--announce-interval", type=int, default=DEFAULT_ANNOUNCE_MINUTES, metavar="MINUTES",
                    help="announce every N minutes (default 60; 0 = on events only)")
    ap.add_argument("-v", "--verbose", action="store_true", help="more stderr (still no foreign text)")
    return ap


def _protocol_stdout() -> Any:
    """The real protocol stream: a private dup of fd 1, with fd 1 pointed at stderr so that anything a
    library prints can never corrupt the protocol."""
    proto = os.fdopen(os.dup(1), "wb", buffering=0)
    os.dup2(2, 1)
    return proto


def main(argv: Optional[List[str]] = None, stdin: Any = None, stdout: Any = None, stderr: Any = None,
         clock: Callable[[], float] = time.monotonic) -> int:
    err = stderr if stderr is not None else sys.stderr
    ap = build_parser()
    try:
        with contextlib.redirect_stderr(err):
            args = ap.parse_args(argv)
    except SystemExit as exc:
        return int(exc.code) if isinstance(exc.code, int) else EXIT_USAGE
    if args.announce_interval < 0 or args.announce_interval > 7 * 24 * 60:
        err.write("lxmf-sidecar: error: --announce-interval must be 0..10080 minutes\n")
        return EXIT_USAGE
    log = Log(err, args.verbose)
    try:
        import importlib
        RNS = importlib.import_module("RNS")
        LXMF = importlib.import_module("LXMF")
    except ImportError:
        log.error("the Python packages rns and lxmf are not installed in this interpreter; run the helper from "
                  "the venv <root>/venvs/lxmf (python3 -I lxmf_sidecar.py ...)")
        return EXIT_NO_LIBS
    if stdout is None:
        stdout = _protocol_stdout()
    if stdin is None:
        stdin = getattr(sys.stdin, "buffer", sys.stdin)
    out = Emitter(stdout, log)
    helper = Helper(args, RNS, LXMF, out, log, clock)
    try:
        helper.attach()
    except StartupError as exc:
        log.error(str(exc))
        if exc.code == EXIT_PARTITIONED:
            out.emit({"type": "link", "state": "partitioned"})
        return exc.code
    except Exception as exc:
        log.error("start-up failed (%s)" % type(exc).__name__)
        return EXIT_START

    inbox: "queue.Queue[Tuple[str, Any]]" = queue.Queue(maxsize=1024)

    def request_stop(signum: int = 0, frame: Any = None) -> None:
        helper.stop.set()
        try:
            inbox.put_nowait(("quit", None))
        except queue.Full:
            pass

    # Install our handlers after Reticulum() installed its own, so SIGTERM ends us cleanly.
    previous: Dict[int, Any] = {}
    if threading.current_thread() is threading.main_thread():
        for sig in (signal.SIGTERM, signal.SIGINT):
            try:
                previous[sig] = signal.signal(sig, request_stop)
            except (ValueError, OSError):
                pass
    code = EXIT_OK
    try:
        helper.start()
        reader = threading.Thread(target=read_commands, args=(stdin, lambda item: inbox.put(item)), daemon=True)
        reader.start()
        helper.run(inbox)
    except Exception as exc:
        log.error("fatal (%s)" % type(exc).__name__)
        code = EXIT_START
    finally:
        for sig, handler in previous.items():
            try:
                signal.signal(sig, handler)
            except (ValueError, OSError):
                pass
        helper.shutdown()
        if helper.malformed:
            log.info("malformed commands ignored: %d" % helper.malformed)
    return code


if __name__ == "__main__":
    rc = main()
    for _stream in (sys.stdout, sys.stderr):
        try:
            _stream.flush()
        except Exception:
            pass
    os._exit(rc)  # library threads must not keep a finished helper alive
