# scripts/notices: inputs for the Layer 5 paths of `gen-notices.py`

Data files, no code. They describe components that are **built or installed outside the Rust workspace**, so Cargo
and pnpm cannot list them. `python3 -I scripts/gen-notices.py` reads them; `--check` hashes them and fails when a
row is missing.

| File | What | When it changes |
|---|---|---|
| `meshmud-<tag>.go.mod`, `meshmud-<tag>.go.sum` | byte-for-byte copies of the pinned upstream tag's `go.mod` / `go.sum` (data, not source). `meshmud-build.sh` compares them with the tarball it builds | when `MESHMUD_TAG` moves |
| `go-licenses.txt` | `module \| version \| SPDX \| copyright \| repository` for every `require` of that go.mod, the main module and `std`. Read each module's `LICENSE` at that version (Go module proxy zip) | with the snapshot; CI fails a binary that links a module not listed |
| `python-licenses.txt` | `name \| version \| SPDX \| copyright \| repository` for every package in `5-messaging/sidecars/meshtastic-presence/meshtastic-presence-lock.txt` | with the lock |
| `tests/` | `build-scripts-gate.sh` (verify paths of the two build scripts against synthetic tarballs), `test_layer5_notices.py` (the generator's Go / Python / pin checks on a temp copy) | |

Pins live in the build scripts, one `NAME="value"` line each (`MESHMUD_*` in `meshmud-build.sh`, `WEB_*` in
`meshtastic-web-build.sh`); the generator hashes those lines, not the whole script.

Moving a pin: fetch the new tarball, update the pin lines, replace the snapshots, update `go-licenses.txt`, run
`meshmud-build.sh --verify-only`, run `gen-notices.py`, review the diff.

Run the tests (from `os/1-bootstrap`):

```sh
sh scripts/notices/tests/build-scripts-gate.sh
python3 -I -B -m unittest discover -s scripts/notices/tests
```
