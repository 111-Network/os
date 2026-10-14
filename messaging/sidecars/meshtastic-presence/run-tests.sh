#!/bin/sh
# Sidecar tests: pytest when the interpreter has it, else unittest. Isolated mode (-I):
# no user site, no PYTHONPATH, no cwd on sys.path. No network, no hardware.
# The real-library tests (tests/test_real_library.py) skip themselves unless meshtastic-python
# is installed; run this script with PYTHON=<presence venv>/bin/python to include them.
set -eu
cd "$(dirname "$0")"
PYTHON=${PYTHON:-python3}
export PYTHONDONTWRITEBYTECODE=1
if "$PYTHON" -I -c 'import pytest' >/dev/null 2>&1; then
  exec "$PYTHON" -I -m pytest -q -p no:cacheprovider tests
fi
exec "$PYTHON" -I -m unittest discover -s tests
