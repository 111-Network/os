# meshtastic-presence

Read-only Meshtastic presence reader for Layer 5 (spec `os/50-messaging/README.md` section 14). It connects to **one**
Meshtastic TCP client-API endpoint (meshmud's TCP downlink, `127.0.0.1:11230`), asks the node for its configuration
once, listens to NodeInfo / position / telemetry events, and writes `<out>/meshtastic.json` atomically for `111d`
to validate and publish as the **Nearby** list. It also has an identify mode (`--once`) that prints the node's own
info for the consented identify step.

**License: GPL-3.0-or-later** (`LICENSE`, SPDX headers), because it imports `meshtastic-python` (GPL-3.0-only).
It is a separate process in its own hash-locked venv; no Rust code imports it and no part of it links into the `111`
binaries.

## Files

| File | What |
|---|---|
| `meshtastic_presence.py` | the program (single file, Python 3.9+, standard library plus `meshtastic`) |
| `requirements.in` | the one top-level pin: `meshtastic==2.7.11` |
| `meshtastic-presence-lock.txt` | the hash-locked closure (16 packages, 335 hashes); `pip install --require-hashes -r` |
| `gen_lock.py` | produces / verifies the lock (needs network to produce) |
| `run-tests.sh`, `tests/` | tests; no network, no hardware |
| `LICENSE` | GPL-3.0 text, unmodified |

## Run

```sh
python3 -I meshtastic_presence.py --endpoint 127.0.0.1:11230 --out /var/lib/111/fabric/presence
python3 -I meshtastic_presence.py --endpoint 127.0.0.1:11230 --once      # identify
```

`--timeout S` (how long to wait for the node database, default 60), `--min-interval S` (minimum seconds between
file writes, default 2), `-v`. Exit codes: 0 stopped by SIGTERM/SIGINT or `--once` succeeded, 2 usage, 3 `--once`
could not identify. Logs go to stderr and never contain node names (RF-origin strings); `111d` captures them.

Behaviour:

* Reconnects forever with exponential back-off (1 s doubling to 60 s, jitter only shortens). Back-off, coalescing
  and liveness polling use the monotonic clock; nothing uses the wall clock.
* Removes a stale `meshtastic.json` at start, so a file from an earlier run (or another board) can never make the
  service look LIVE. The first file appears when the node database has been downloaded, even if it holds no other
  node (`{"schema":1,"nodes":[]}`).
* Rewrites the file only when its content changed, at most every `--min-interval` seconds.
* A SIGTERM during a blocking connect aborts it immediately.

## Output: `meshtastic.json` (spec section 14, schema 1)

```json
{"schema":1,"nodes":[{"id":"!1b2c3d4e","num":456789326,"long_name":"Hilltop","short_name":"HT",
  "pubkey":"<base64>","snr":5.5,"rssi":-97,"hops_away":1,"last_heard_hint":1760000000,
  "position":{"lat":51.5074,"lon":-0.1278,"alt":35},"channel_utilization":7.25}]}
```

| Field | Rule |
|---|---|
| `id`, `num` | always present. `num` is the node number (1 .. 2^32-2); `id` is rebuilt as `!` + 8 lowercase hex digits, **never copied from the radio** |
| `long_name`, `short_name` | always present, possibly `""`. Control characters (Cc), surrogates, line/paragraph separators and bidi overrides stripped; trimmed; cut to **40 UTF-8 bytes** on a code point boundary |
| `pubkey` | optional; standard base64 of exactly 32 bytes, else omitted |
| `snr` (dB, -100..100), `rssi` (dBm, -200..0), `hops_away` (0..7), `channel_utilization` (percent, 0..100) | optional, range-checked, dropped when out of range / not a real number |
| `last_heard_hint` | optional; the **node's own clock** (unix seconds), display only, never used to order or authenticate |
| `position` | optional `{lat, lon, alt?}`; lat/lon degrees to 7 places, `alt` metres; omitted without a fix (0,0 is "no fix") |

Limits: at most **4096 nodes** and at most **1,000,000 bytes** (daemon limit is 1 MiB); when rows must go, the least
recently heard are dropped first. Rows are written in ascending `num` order. The local board is **excluded** (it is
not "heard"); identify reports it. Unknown or hostile input never raises: it is dropped field by field.

### `--once` output

One JSON line on stdout, nothing written to disk:

```json
{"schema":1,"my_node_info":{"num":168430090,"id":"!0a0a0a0a","long_name":"My Board","short_name":"MB",
 "hw_model":"HELTEC_V3","firmware_version":"2.7.11.abcdef0","pio_env":"heltec-v3","pubkey":"<base64>"}}
```

`pio_env` and `pubkey` are optional. `firmware_version` feeds `DeviceRecord.firmware.version`.

## No send path

111 never transmits on the Meshtastic mesh and never changes node configuration. This program enforces it at the
lowest layer, not by convention:

* It uses a subclass of the library's `TCPInterface` (`ReadOnlyTCPInterface`) whose `_sendToRadio` lets exactly
  one `ToRadio` variant through, **once per connection**: `want_config_id`. Anything else (text, data, admin,
  position, traceroute, any `MeshPacket`) is refused and counted, whoever asks, including the library itself.
* The library's `disconnect` message on close and its 5-minute `heartbeat` are removed too: meshmud forwards
  downlink frames to the board verbatim, and a `disconnect` would end the board's API session for every other client.
* It skips the 32 resync bytes meant for sleeping serial devices, and never re-dials by itself (a reconnect inside
  the library would re-send `want_config` outside our back-off).
* `tests/test_presence.py` proves it by scanning the source for every send/write/config API name (none appears) and
  by behaviour; `tests/test_real_library.py` proves against the pinned library, with a fake node on a loopback
  socket, that only `want_config_id` ever reaches the wire, at start, during events, at close and across reconnects.

What the user does in the *panel* is a different path: it goes to their radio and is neither seen nor stored here.

## Deploy

The release carries this directory and the lock. On the node, `111d` builds the venv offline-first like the rns venv:

```sh
python3 -m venv /var/lib/111/venvs/presence
/var/lib/111/venvs/presence/bin/pip install --require-hashes -r meshtastic-presence-lock.txt
```

and supervises `venvs/presence/bin/python -I <dir>/meshtastic_presence.py --endpoint 127.0.0.1:11230 --out
/var/lib/111/fabric/presence` (restart after meshmud). Nothing is vendored; nothing downloads except that pip step on
the user's own machine (or from the installer's prefetched cache).

### Regenerate the lock (release time, needs network)

```sh
python3 gen_lock.py --pip "python3 -m pip"      # resolves requirements.in, hashes every published file
python3 gen_lock.py --verify                    # offline structural check
```

Resolve on the **oldest supported CPython (3.10)**, Linux. The closure of `meshtastic` is 16 packages including a BLE
stack (`bleak`, and `dbus-fast` on Linux) that the reader never imports. A package that every dependency edge reaches
only through a marker keeps that marker (`async-timeout`, `typing-extensions`, `dbus-fast`). After changing the pin,
also update `os/10-bootstrap/scripts/notices/python-licenses.txt` and run `python3 -I os/10-bootstrap/scripts/gen-notices.py`.

**Termux (external gate, spec section 20):** `dbus-fast` ships compiled wheels for glibc/musl only; on Android pip may
fall back to a source build, which the hash lock does not cover. Check it with the Termux venv build before the first
signed release.

## Tests

```sh
./run-tests.sh                                  # pytest if present, else unittest; python3 -I
PYTHON=/var/lib/111/venvs/presence/bin/python ./run-tests.sh      # also runs the real-library tests
```

Without `meshtastic-python` the real-library tests (`tests/test_real_library.py`) skip themselves; every other test runs. The lock test (`tests/test_lock_and_manifest.py`)
also checks the panel declaration `10-bootstrap/crates/node111/assets/panels/meshtastic.toml` against the spec section 13 block
(line-grammar parser on Python < 3.11, `tomllib` / `tomli` when available).
