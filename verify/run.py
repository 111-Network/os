#!/usr/bin/env python3
"""The cross-layer test runner: ``python3 -I verify/run.py --scratch DIR [selection] [flags]`` (D36, D37).

One entry point over the repo's existing gates and harnesses; ``--help`` lists every flag, ``verify/t111/`` holds the
code (``t111/cli.py`` is the command line, ``t111/schema.py`` the map's data model). Repo-only tooling: standard
library, Python >= 3.11, nothing installed on a node.
"""

import sys

if sys.version_info < (3, 11):
    sys.stderr.write("verify/run.py needs Python >= 3.11 (tomllib)\n")
    sys.exit(2)

sys.dont_write_bytecode = True  # a run writes nothing into the tree, __pycache__ included

from pathlib import Path  # noqa: E402

# -I drops the script's directory from sys.path; add verify/ back explicitly so `t111` imports.
sys.path.insert(0, str(Path(__file__).resolve().parent))

from t111.cli import main  # noqa: E402

if __name__ == "__main__":
    sys.exit(main())
