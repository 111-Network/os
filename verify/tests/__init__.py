"""The runner's own hermetic tests: ``python3 -I -m unittest discover -s verify/tests -t .`` from the repo root.

No test runs a gate of the repo: every check is a tiny script written into a temp git repo (the stand-in "tree"), every
run's scratch and lock dir live in a temp dir, and the fixture map is ``verify/tests/fixtures/map/``.
"""

import sys
from pathlib import Path

sys.dont_write_bytecode = True
_VERIFY = str(Path(__file__).resolve().parent.parent)
if _VERIFY not in sys.path:
    sys.path.insert(0, _VERIFY)
