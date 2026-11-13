"""t111: the cross-layer test runner behind ``python3 -I verify/run.py`` (D36, D37).

The package is repo-only tooling (the Pi floor): standard library, no install, nothing in ``111``.

Modules (row ownership in ``plan/cross-layer/test-system/TASKS.md``):

- ``schema``   the one definition of the seam map's data model and its loader (X2)
- ``select``   which checks a command line selects, in run order (X2)
- ``locks``    the cargo slot locks and the named exclusive locks (X2)
- ``run``      one check as one subprocess session (X2)
- ``hygiene``  processes, listeners, the tree and capability probes around a check (X2)
- ``record``   the JSON-lines result records and their enrichment from the map (X2)
- ``summary``  ``summary.txt``, the last line and the exit code (X2)
- ``engine``   the scheduler that ties the above together for one run (X2)
- ``cli``      the argument parser and the command dispatch behind ``verify/run.py`` (X2)
- ``adapters`` the adapter registry and the trivial ``exit`` adapter (X2); the real adapters (X3)
- ``lint``, ``matrix``, ``changed``, ``explain``: the map commands (X4)
"""
