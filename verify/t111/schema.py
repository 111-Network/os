"""The seam map's data model: the ONE definition of ``verify/seams.toml`` and ``verify/checks.toml``.

This file is the schema's documentation (``plan/cross-layer/test-system/options.md`` §3.2 and §3.5). Every TOML field
the runner reads is a dataclass field below with its meaning in the class docstring; a field the loader does not know is
an error, so a typo never passes silently.

``verify/seams.toml`` holds one ``[[seam]]`` table per seam, sorted by id::

    [[seam]]
    id = "L3-L4:chat-relay"
    status = "built"                     # built | stub | planned
    spec = ["4-interface/README.md §10", "4-interface/api/README.md"]
    a = { layer = 4, files = ["1-bootstrap/crates/node111/src/daemon/dashboard/api_v1/relay.rs"] }
    b = { layer = 3, files = ["3-intelligence/crates/agent111/src/http.rs"] }
    read_first = "a"
    symptom = "UI error banner with a problem code; the stream ends without done"
    gap = ""

``verify/checks.toml`` holds ``[[check]]``, ``[[fake]]`` and ``[[known]]`` tables and one ``[unmapped]`` table
(``"<path>" = "<reason>"``).

Module contents: the enum tuples, the dataclasses ``Side``, ``Seam``, ``Check``, ``Fake``, ``Known``, ``Unmapped``,
``SeamMap``, the error types ``MapError`` / ``SchemaError`` and the loaders ``load_map``, ``load_files``, ``parse_map``.
The loader checks types, enums, unique ids and the check-to-check references the runner depends on (``covered_by``,
``requires``, cycles). Everything that needs the tree (files exist, a check names a known seam, a section heading
resolves) is the lint's job (``t111.lint``, row X4).
"""

from __future__ import annotations

import re
import tomllib
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any, Mapping

SEAM_STATUS: tuple[str, ...] = ("built", "stub", "planned")
"""``seam.status``: ``built`` (both sides exist), ``stub`` (a side is a T0 stub), ``planned`` (no code yet)."""

CHECK_KINDS: tuple[str, ...] = ("gate", "unit")
"""``check.kind``: a ``gate`` runs a layer's own script as is; a ``unit`` runs one test binary, file or harness."""

TIERS: tuple[str, ...] = ("fast", "slow", "ns")
"""``check.tier``: ``fast`` seconds, no build; ``slow`` a cargo or pnpm build or suite; ``ns`` namespaces or root."""

DEFAULT_TIERS: tuple[str, ...] = ("fast", "slow")
"""The tiers a run selects when ``--tier`` is not given; ``ns`` is opt-in."""

STRENGTHS: tuple[str, ...] = ("R", "I", "F", "C", "N")
"""``check.strength`` (matrix.md legend): R running system, I integration with the real peer in one process, F against a
fake of the other side, C compiled or static only, N none."""

SIDES: tuple[str, ...] = ("a", "b")
"""The two sides of a seam; ``seam.read_first`` and ``fake.mirrors`` name one of them."""

KNOWN_CONDITIONS: tuple[str, ...] = ("euid0", "nonroot")
"""``known.condition``: ``euid0`` holds when the runner's effective uid is 0; ``nonroot`` when it is not."""

KNOWN_KINDS: tuple[str, ...] = ("fails", "vacuous")
"""``known.kind``: ``fails`` = the test fails under the condition (FAIL becomes SKIPPED, PASS stays PASS);
``vacuous`` = the test returns early under the condition and would count PASS for nothing (PASS becomes SKIPPED)."""

CHECK_ID_RE = re.compile(r"^[a-z0-9][a-z0-9._-]*$")
"""A check id: lower case, digits, ``.``, ``_``, ``-``; no ``/`` (record ids are ``<check>/<…>``)."""

RESERVED_CHECK_IDS: frozenset[str] = frozenset({"runner", "run"})
"""Check ids the engine's own records use (``runner``, ``hygiene/run``): a check may not take them."""

SEAM_ID_RE = re.compile(r"^(?:L\d(?:/\d)*-(?:L\d(?:/\d)*|X)|X):[a-z0-9][a-z0-9.-]*$")
"""A seam id: ``L<a>-L<b>:<name>``, ``L<a>-X:<name>`` or ``X:<name>`` (``L0-L5/6/7:entries`` is allowed)."""

LAYER_X = "X"
"""The cross-cutting pseudo layer (a side or a check that belongs to no single layer)."""

Layer = int | str
"""A layer: an int 0..9 or ``"X"``."""


@dataclass(frozen=True)
class Side:
    """One side of a seam.

    - ``layer``: the layer that owns this side: an int 0..9, ``"X"``, or a list of ints when one side spans layers
      (``L0-L5/6/7:entries``); kept as a tuple of layers.
    - ``files``: repo-relative paths of the code (or docs) on this side; the record's ``sides`` and the summary's
      ``read first`` line print them.
    """

    layer: tuple[Layer, ...]
    files: tuple[str, ...]

    def to_json(self) -> dict[str, Any]:
        """The record form: ``{"layer": <int|str|list>, "files": [...]}``."""
        layer: Any = self.layer[0] if len(self.layer) == 1 else list(self.layer)
        return {"layer": layer, "files": list(self.files)}


@dataclass(frozen=True)
class Seam:
    """One ``[[seam]]`` of ``verify/seams.toml``.

    - ``id``: the seam id of ``research/05-seams.md`` (``L3-L4:chat-relay``); unique.
    - ``status``: one of ``SEAM_STATUS``.
    - ``spec``: spec references (``"4-interface/README.md §10"``); the lint resolves each ``§N.M`` to a heading.
    - ``a``, ``b``: the two sides (``Side``); ``a`` is conventionally the lower layer's side.
    - ``read_first``: ``"a"`` or ``"b"``, the producer of the data the assertions read: where a running-system failure
      points first.
    - ``symptom``: what a daemon log, an API answer or the UI shows when this seam breaks.
    - ``gap``: required text when the seam is built and has no check of strength R (the lint enforces it); empty
      otherwise.
    """

    id: str
    status: str
    spec: tuple[str, ...]
    a: Side
    b: Side
    read_first: str
    symptom: str = ""
    gap: str = ""

    def side(self, name: str) -> Side:
        """Side ``"a"`` or ``"b"``."""
        return self.a if name == "a" else self.b

    def layers(self) -> tuple[Layer, ...]:
        """The layers this seam joins, a side first then b, without repeats."""
        out: list[Layer] = []
        for layer in self.a.layer + self.b.layer:
            if layer not in out:
                out.append(layer)
        return tuple(out)


@dataclass(frozen=True)
class Check:
    """One ``[[check]]`` of ``verify/checks.toml``.

    - ``id``: unique, ``CHECK_ID_RE`` (``l1.gate``, ``l4.contract.fresh``).
    - ``kind``: ``gate`` or ``unit`` (``CHECK_KINDS``).
    - ``cmd``: the argv list (never a shell string). Tokens ``{root}``, ``{scratch}``, ``{tmp}`` and ``{run}`` are
      replaced by the repo root, the check's scratch dir, its ``TMPDIR`` and the run dir.
    - ``cwd``: repo-relative working directory (default ``"."``).
    - ``tier``: ``fast`` | ``slow`` | ``ns`` (``TIERS``).
    - ``layers``: the layers the check touches (ints 0..9 or ``"X"``); ``--layer`` selects on it.
    - ``seams``: the seam ids the check's results prove by default.
    - ``phases``: phase prefix -> seam ids; an adapter that sees ``== <phase>`` maps the records of that phase to those
      seams instead of ``seams``.
    - ``adapter``: the name in ``t111.adapters.REGISTRY`` that turns the log into records (default ``exit``).
    - ``cargo``: true when the check builds with cargo; it then takes one of ``--cargo-slots`` slot locks.
    - ``lock``: the name of an exclusive lock (``"verify"``) or empty; every ``ns`` check takes ``verify`` anyway.
    - ``timeout_s``: the hard bound in seconds; at expiry the group gets TERM, then KILL after 10 s (required).
    - ``env``: extra environment (the same tokens as ``cmd`` are replaced in the values).
    - ``rerun``: the one-check re-run command template; ``{id}``, ``{test}``, ``{check}``, ``{root}`` are filled per
      record. Empty means the ``cmd`` itself (run from ``cwd``).
    - ``rerun_cost``: how much the re-run costs, as words (``"slow, ~20 s warm"``, ``"ns, ~4 min"``).
    - ``covered_by``: a unit's covering gate id; in a full or ``--gate`` run the gate runs instead of the unit (no
      suite runs twice). Empty for gates and for units no gate covers.
    - ``strength``: ``R`` | ``I`` | ``F`` | ``C`` | ``N`` (``STRENGTHS``): the evidence the check gives its seams.
    - ``requires``: ids of checks that must have passed first; a unit whose prerequisite did not pass is SKIPPED
      ``requires <id> (<status>)``. A selected check pulls its prerequisites into the plan.
    - ``needs``: tools or capabilities the check needs: ``userns``, ``netns`` (probed with ``unshare -Urnm true``),
      ``root``, ``nonroot``, ``network`` (absent under ``--offline``), or any executable name (looked up on PATH). A
      missing need makes the check SKIPPED ``needs <x> (missing)``.
    - ``offline_skip``: true when the check needs the package index; ``--offline`` makes it SKIPPED ``offline``.
    """

    id: str
    kind: str
    cmd: tuple[str, ...]
    tier: str
    layers: tuple[Layer, ...]
    strength: str
    timeout_s: int
    cwd: str = "."
    seams: tuple[str, ...] = ()
    phases: Mapping[str, tuple[str, ...]] = field(default_factory=dict)
    adapter: str = "exit"
    cargo: bool = False
    lock: str = ""
    env: Mapping[str, str] = field(default_factory=dict)
    rerun: str = ""
    rerun_cost: str = ""
    covered_by: str = ""
    requires: tuple[str, ...] = ()
    needs: tuple[str, ...] = ()
    offline_skip: bool = False

    def locks(self) -> tuple[str, ...]:
        """The exclusive named locks this check takes, sorted (``ns`` implies ``verify``)."""
        names = set()
        if self.lock:
            names.add(self.lock)
        if self.tier == "ns":
            names.add("verify")
        return tuple(sorted(names))


@dataclass(frozen=True)
class Fake:
    """One ``[[fake]]``: a stub or fake that stands in for one side of a seam.

    - ``path``: the fake's repo-relative path.
    - ``mirrors``: the side it stands in for (``"a"`` | ``"b"``); the other side is the one under test.
    - ``seam``: the seam id.
    - ``parity``: the check id that compares the fake with what it mirrors, or ``"none"`` (counted as a gap).
    """

    path: str
    mirrors: str
    seam: str
    parity: str = "none"


@dataclass(frozen=True)
class Known:
    """One ``[[known]]`` (options.md F7): a test result that is known under a condition.

    - ``test``: the record id it matches; ``fnmatch`` wildcards are allowed (``l1.node111/api_v1_relay/*``).
    - ``condition``: when it applies (``KNOWN_CONDITIONS``: ``euid0``, ``nonroot``).
    - ``source``: where it is recorded (``6-transport/TASKS.md:80``).
    - ``reason``: the words after ``known:`` in the record (default ``fails as root`` / ``fails as non-root`` /
      ``returns early ...`` by condition and kind).
    - ``check``: optional; restricts the match to records of this check id.
    - ``kind``: ``fails`` (default) or ``vacuous`` (``KNOWN_KINDS``).

    The rule: when the condition holds, a matching FAIL (``fails``) or PASS (``vacuous``) is recorded SKIPPED with the
    reason ``known: <reason> (<source>)``; a ``fails`` test that passes counts PASS; ``--require`` makes it FAIL.
    """

    test: str
    condition: str
    source: str
    reason: str = ""
    check: str = ""
    kind: str = "fails"

    def words(self) -> str:
        """The reason text without the ``known:`` prefix."""
        if self.reason:
            return self.reason
        who = "root" if self.condition == "euid0" else "non-root"
        if self.kind == "vacuous":
            return f"returns early as {who}, a PASS here proves nothing"
        return f"fails as {who}"


@dataclass(frozen=True)
class Unmapped:
    """One entry of ``[unmapped]``: a test or harness file tied to no seam on purpose.

    - ``path``: the repo-relative file.
    - ``reason``: why it maps to no seam.
    """

    path: str
    reason: str


@dataclass(frozen=True)
class SeamMap:
    """The loaded map: every table of both files, in file order, with id lookups."""

    seams: tuple[Seam, ...]
    checks: tuple[Check, ...]
    fakes: tuple[Fake, ...] = ()
    known: tuple[Known, ...] = ()
    unmapped: tuple[Unmapped, ...] = ()

    def seam(self, seam_id: str) -> Seam | None:
        """The seam with this id, or None."""
        for seam in self.seams:
            if seam.id == seam_id:
                return seam
        return None

    def check(self, check_id: str) -> Check | None:
        """The check with this id, or None."""
        for check in self.checks:
            if check.id == check_id:
                return check
        return None

    def fakes_on(self, seam_id: str) -> tuple[Fake, ...]:
        """The fakes declared on one seam."""
        return tuple(f for f in self.fakes if f.seam == seam_id)


@dataclass(frozen=True)
class MapError:
    """One schema error: the file, the table (``check``), its 0-based index, the id when known, the field, the text."""

    file: str
    table: str
    index: int | None
    field: str
    message: str
    id: str = ""

    def __str__(self) -> str:
        if not self.table:
            return f"{self.file}: {self.message}"
        where = self.table if self.index is None else f"{self.table}[{self.index}]"
        if self.id:
            where += f" ({self.id})"
        if self.field:
            where += f".{self.field}"
        return f"{self.file}: {where}: {self.message}"


class SchemaError(Exception):
    """The map does not load; ``errors`` lists every problem found (not only the first)."""

    def __init__(self, errors: list[MapError]):
        super().__init__("\n".join(str(e) for e in errors))
        self.errors = errors


# ---------------------------------------------------------------------------------------------------------------------
# loader


class _Table:
    """Reads one TOML table and collects errors with file, table, index and field."""

    def __init__(self, errors: list[MapError], file: str, table: str, index: int | None, raw: Any):
        self.errors = errors
        self.file = file
        self.table = table
        self.index = index
        self.raw: dict[str, Any] = raw if isinstance(raw, dict) else {}
        self.id = str(self.raw.get("id", "")) if isinstance(self.raw.get("id", ""), str) else ""
        self.ok = True
        if not isinstance(raw, dict):
            self.err("", "is not a table")

    def err(self, fld: str, message: str) -> None:
        self.ok = False
        self.errors.append(MapError(self.file, self.table, self.index, fld, message, self.id))

    def unknown(self, allowed: set[str]) -> None:
        for key in self.raw:
            if key not in allowed:
                self.err(key, f"unknown field (allowed: {', '.join(sorted(allowed))})")

    def str_(self, fld: str, *, required: bool = False, default: str = "", choices: tuple[str, ...] = ()) -> str:
        if fld not in self.raw:
            if required:
                self.err(fld, "missing (required)")
            return default
        value = self.raw[fld]
        if not isinstance(value, str):
            self.err(fld, f"must be a string, got {type(value).__name__}")
            return default
        if choices and value not in choices:
            self.err(fld, f"{value!r} is not one of {'|'.join(choices)}")
            return default
        return value

    def bool_(self, fld: str, default: bool = False) -> bool:
        if fld not in self.raw:
            return default
        value = self.raw[fld]
        if not isinstance(value, bool):
            self.err(fld, f"must be true or false, got {type(value).__name__}")
            return default
        return value

    def int_(self, fld: str, *, required: bool = False, minimum: int = 1) -> int:
        if fld not in self.raw:
            if required:
                self.err(fld, "missing (required)")
            return 0
        value = self.raw[fld]
        if isinstance(value, bool) or not isinstance(value, int):
            self.err(fld, f"must be an integer, got {type(value).__name__}")
            return 0
        if value < minimum:
            self.err(fld, f"must be >= {minimum}, got {value}")
            return 0
        return value

    def strs(self, fld: str, *, required: bool = False, nonempty: bool = False) -> tuple[str, ...]:
        if fld not in self.raw:
            if required:
                self.err(fld, "missing (required)")
            return ()
        value = self.raw[fld]
        if not isinstance(value, list) or not all(isinstance(v, str) for v in value):
            self.err(fld, "must be a list of strings")
            return ()
        if nonempty and not value:
            self.err(fld, "must not be empty")
        if len(set(value)) != len(value):
            self.err(fld, "has a repeated entry")
        return tuple(value)

    def layers(self, fld: str, value: Any, *, allow_list: bool) -> tuple[Layer, ...]:
        def one(v: Any) -> Layer | None:
            if isinstance(v, bool):
                return None
            if isinstance(v, int) and 0 <= v <= 9:
                return v
            if v == LAYER_X:
                return v
            return None

        if isinstance(value, list) and allow_list:
            got = [one(v) for v in value]
            if any(g is None for g in got):
                self.err(fld, f"layers must be ints 0..9 or \"X\", got {value!r}")
                return ()
            return tuple(g for g in got if g is not None)
        single = one(value)
        if single is None:
            self.err(fld, f"must be an int 0..9 or \"X\"{' or a list of them' if allow_list else ''}, got {value!r}")
            return ()
        return (single,)


def _side(t: _Table, name: str) -> Side:
    raw = t.raw.get(name)
    if raw is None:
        t.err(name, "missing (required)")
        return Side((), ())
    if not isinstance(raw, dict):
        t.err(name, "must be an inline table { layer = N, files = [...] }")
        return Side((), ())
    for key in raw:
        if key not in ("layer", "files"):
            t.err(f"{name}.{key}", "unknown field (allowed: files, layer)")
    if "layer" not in raw:
        t.err(f"{name}.layer", "missing (required)")
        layer: tuple[Layer, ...] = ()
    else:
        layer = t.layers(f"{name}.layer", raw["layer"], allow_list=True)
    files = raw.get("files", [])
    if not isinstance(files, list) or not all(isinstance(f, str) for f in files):
        t.err(f"{name}.files", "must be a list of strings")
        files = []
    return Side(layer, tuple(files))


def _parse_seam(errors: list[MapError], file: str, index: int, raw: Any) -> Seam | None:
    t = _Table(errors, file, "seam", index, raw)
    if not isinstance(raw, dict):
        return None
    t.unknown({"id", "status", "spec", "a", "b", "read_first", "symptom", "gap"})
    sid = t.str_("id", required=True)
    if sid and not SEAM_ID_RE.match(sid):
        t.err("id", f"{sid!r} is not a seam id (L<a>-L<b>:<name>, L<a>-X:<name> or X:<name>)")
    seam = Seam(
        id=sid,
        status=t.str_("status", required=True, choices=SEAM_STATUS),
        spec=t.strs("spec"),
        a=_side(t, "a"),
        b=_side(t, "b"),
        read_first=t.str_("read_first", required=True, choices=SIDES),
        symptom=t.str_("symptom"),
        gap=t.str_("gap"),
    )
    return seam if t.ok else None


_CHECK_FIELDS = {
    "id", "kind", "cmd", "cwd", "tier", "layers", "seams", "phases", "adapter", "cargo", "lock", "timeout_s", "env",
    "rerun", "rerun_cost", "covered_by", "strength", "requires", "needs", "offline_skip",
}


def _parse_check(errors: list[MapError], file: str, index: int, raw: Any) -> Check | None:
    t = _Table(errors, file, "check", index, raw)
    if not isinstance(raw, dict):
        return None
    t.unknown(_CHECK_FIELDS)
    cid = t.str_("id", required=True)
    if cid and not CHECK_ID_RE.match(cid):
        t.err("id", f"{cid!r} is not a check id ([a-z0-9][a-z0-9._-]*)")
    elif cid in RESERVED_CHECK_IDS:
        t.err("id", f"{cid!r} is reserved for the engine's own records (runner, hygiene/run)")
    kind =t.str_("kind", required=True, choices=CHECK_KINDS)
    cmd = t.strs("cmd", required=True, nonempty=False)
    if "cmd" in t.raw and not cmd and isinstance(t.raw["cmd"], list):
        t.err("cmd", "must not be empty")
    layers: tuple[Layer, ...] = ()
    if "layers" not in t.raw:
        t.err("layers", "missing (required)")
    elif not isinstance(t.raw["layers"], list):
        t.err("layers", "must be a list of ints 0..9 or \"X\"")
    else:
        layers = t.layers("layers", t.raw["layers"], allow_list=True)
    phases: dict[str, tuple[str, ...]] = {}
    raw_phases = t.raw.get("phases", {})
    if not isinstance(raw_phases, dict):
        t.err("phases", "must be a table { \"<phase prefix>\" = [\"<seam id>\", ...] }")
    else:
        for prefix, seams in raw_phases.items():
            if not isinstance(seams, list) or not all(isinstance(s, str) for s in seams):
                t.err(f"phases.{prefix}", "must be a list of seam ids")
            else:
                phases[prefix] = tuple(seams)
    env: dict[str, str] = {}
    raw_env = t.raw.get("env", {})
    if not isinstance(raw_env, dict):
        t.err("env", "must be a table of strings")
    else:
        for key, value in raw_env.items():
            if not isinstance(value, str):
                t.err(f"env.{key}", "must be a string")
            elif not re.match(r"^[A-Za-z_][A-Za-z0-9_]*$", key):
                t.err(f"env.{key}", "is not an environment variable name")
            else:
                env[key] = value
    covered_by = t.str_("covered_by")
    if covered_by and kind == "gate":
        t.err("covered_by", "a gate is covered by nothing (only a unit names its covering gate)")
    lock = t.str_("lock")
    if lock and not CHECK_ID_RE.match(lock):
        t.err("lock", f"{lock!r} is not a lock name ([a-z0-9][a-z0-9._-]*)")
    check = Check(
        id=cid,
        kind=kind,
        cmd=cmd,
        cwd=t.str_("cwd", default="."),
        tier=t.str_("tier", required=True, choices=TIERS),
        layers=layers,
        seams=t.strs("seams"),
        phases=phases,
        adapter=t.str_("adapter", default="exit"),
        cargo=t.bool_("cargo"),
        lock=lock,
        timeout_s=t.int_("timeout_s", required=True),
        env=env,
        rerun=t.str_("rerun"),
        rerun_cost=t.str_("rerun_cost"),
        covered_by=covered_by,
        strength=t.str_("strength", required=True, choices=STRENGTHS),
        requires=t.strs("requires"),
        needs=t.strs("needs"),
        offline_skip=t.bool_("offline_skip"),
    )
    if check.cwd.startswith("/") or ".." in Path(check.cwd).parts:
        t.err("cwd", "must be a repo-relative path inside the tree")
    return check if t.ok else None


def _parse_fake(errors: list[MapError], file: str, index: int, raw: Any) -> Fake | None:
    t = _Table(errors, file, "fake", index, raw)
    if not isinstance(raw, dict):
        return None
    t.unknown({"path", "mirrors", "seam", "parity"})
    fake = Fake(
        path=t.str_("path", required=True),
        mirrors=t.str_("mirrors", required=True, choices=SIDES),
        seam=t.str_("seam", required=True),
        parity=t.str_("parity", default="none") or "none",
    )
    return fake if t.ok else None


def _parse_known(errors: list[MapError], file: str, index: int, raw: Any) -> Known | None:
    t = _Table(errors, file, "known", index, raw)
    if not isinstance(raw, dict):
        return None
    t.unknown({"test", "condition", "source", "reason", "check", "kind"})
    known = Known(
        test=t.str_("test", required=True),
        condition=t.str_("condition", required=True, choices=KNOWN_CONDITIONS),
        source=t.str_("source", required=True),
        reason=t.str_("reason"),
        check=t.str_("check"),
        kind=t.str_("kind", default="fails", choices=KNOWN_KINDS),
    )
    return known if t.ok else None


def _array(errors: list[MapError], file: str, doc: dict[str, Any], name: str) -> list[Any]:
    value = doc.get(name, [])
    if not isinstance(value, list):
        errors.append(MapError(file, name, None, "", f"must be an array of tables ([[{name}]])"))
        return []
    return value


def parse_map(seams_doc: Mapping[str, Any], checks_doc: Mapping[str, Any], *, seams_file: str = "verify/seams.toml",
              checks_file: str = "verify/checks.toml") -> SeamMap:
    """Build a ``SeamMap`` from two parsed TOML documents; raise ``SchemaError`` listing every problem."""
    errors: list[MapError] = []
    for key in seams_doc:
        if key != "seam":
            errors.append(MapError(seams_file, key, None, "", "unknown top-level table (allowed: seam)"))
    for key in checks_doc:
        if key not in ("check", "fake", "known", "unmapped"):
            errors.append(MapError(checks_file, key, None, "", "unknown top-level table (allowed: check, fake, known, "
                                                               "unmapped)"))
    seams = [s for i, raw in enumerate(_array(errors, seams_file, dict(seams_doc), "seam"))
             if (s := _parse_seam(errors, seams_file, i, raw)) is not None]
    checks = [c for i, raw in enumerate(_array(errors, checks_file, dict(checks_doc), "check"))
              if (c := _parse_check(errors, checks_file, i, raw)) is not None]
    fakes = [f for i, raw in enumerate(_array(errors, checks_file, dict(checks_doc), "fake"))
             if (f := _parse_fake(errors, checks_file, i, raw)) is not None]
    known = [k for i, raw in enumerate(_array(errors, checks_file, dict(checks_doc), "known"))
             if (k := _parse_known(errors, checks_file, i, raw)) is not None]
    unmapped: list[Unmapped] = []
    raw_unmapped = checks_doc.get("unmapped", {})
    if not isinstance(raw_unmapped, dict):
        errors.append(MapError(checks_file, "unmapped", None, "", "must be one table of \"<path>\" = \"<reason>\""))
    else:
        for path, reason in raw_unmapped.items():
            if not isinstance(reason, str) or not reason.strip():
                errors.append(MapError(checks_file, "unmapped", None, path, "needs a reason (a non-empty string)"))
            else:
                unmapped.append(Unmapped(path, reason))

    _unique(errors, seams_file, "seam", [s.id for s in seams])
    _unique(errors, checks_file, "check", [c.id for c in checks])
    by_id = {c.id: c for c in checks}
    for index, check in enumerate(checks):
        if check.covered_by:
            gate = by_id.get(check.covered_by)
            if gate is None:
                errors.append(MapError(checks_file, "check", index, "covered_by",
                                       f"names no check: {check.covered_by!r}", check.id))
            elif gate.kind != "gate":
                errors.append(MapError(checks_file, "check", index, "covered_by",
                                       f"{check.covered_by!r} is a unit, not a gate", check.id))
        for req in check.requires:
            if req not in by_id:
                errors.append(MapError(checks_file, "check", index, "requires", f"names no check: {req!r}", check.id))
            elif req == check.id:
                errors.append(MapError(checks_file, "check", index, "requires", "names the check itself", check.id))
    cycle = _requires_cycle(checks)
    if cycle:
        errors.append(MapError(checks_file, "check", None, "requires", "cycle: " + " -> ".join(cycle)))
    if errors:
        raise SchemaError(errors)
    return SeamMap(tuple(seams), tuple(checks), tuple(fakes), tuple(known), tuple(unmapped))


def _unique(errors: list[MapError], file: str, table: str, ids: list[str]) -> None:
    seen: dict[str, int] = {}
    for index, ident in enumerate(ids):
        if ident in seen:
            errors.append(MapError(file, table, index, "id", f"duplicate id (first at {table}[{seen[ident]}])", ident))
        else:
            seen[ident] = index


def _requires_cycle(checks: list[Check]) -> list[str]:
    graph = {c.id: [r for r in c.requires if r != c.id] for c in checks}
    state: dict[str, int] = {}
    stack: list[str] = []

    def visit(node: str) -> list[str]:
        state[node] = 1
        stack.append(node)
        for nxt in graph.get(node, []):
            if nxt not in graph:
                continue
            if state.get(nxt) == 1:
                return stack[stack.index(nxt):] + [nxt]
            if state.get(nxt) is None:
                found = visit(nxt)
                if found:
                    return found
        stack.pop()
        state[node] = 2
        return []

    for node in graph:
        if state.get(node) is None:
            found = visit(node)
            if found:
                return found
    return []


def _read_toml(path: Path, shown: str, errors: list[MapError]) -> dict[str, Any]:
    try:
        with path.open("rb") as fh:
            return tomllib.load(fh)
    except FileNotFoundError:
        errors.append(MapError(shown, "", None, "", "file not found"))
    except tomllib.TOMLDecodeError as exc:
        errors.append(MapError(shown, "", None, "", f"not valid TOML: {exc}"))
    return {}


def load_files(seams_path: Path, checks_path: Path, *, display_root: Path | None = None) -> SeamMap:
    """Load the two map files; errors name each file relative to ``display_root`` when given."""

    def shown(path: Path) -> str:
        if display_root is not None:
            try:
                return str(path.resolve().relative_to(display_root.resolve()))
            except ValueError:
                pass
        return str(path)

    errors: list[MapError] = []
    seams_doc = _read_toml(seams_path, shown(seams_path), errors)
    checks_doc = _read_toml(checks_path, shown(checks_path), errors)
    if errors:
        raise SchemaError(errors)
    return parse_map(seams_doc, checks_doc, seams_file=shown(seams_path), checks_file=shown(checks_path))


def load_map(map_dir: Path, *, display_root: Path | None = None) -> SeamMap:
    """Load ``<map_dir>/seams.toml`` and ``<map_dir>/checks.toml`` (``map_dir`` is ``verify/`` by default)."""
    return load_files(map_dir / "seams.toml", map_dir / "checks.toml", display_root=display_root)
