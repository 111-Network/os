# lxmf-sidecar

The Layer 6 LXMF helper (spec `os/60-transport/README.md` §6, D26 #4): the one program of ours that imports Reticulum. It
attaches to the node's shared `rnsd` instance as an interface-less client, owns one LXMF delivery identity, and speaks JSON
Lines over piped stdio to `111d` (stdout is the protocol, stderr the log). It never listens on a port.

**Credit.** Reticulum and LXMF are the work of the Reticulum project (reticulum.network); 111 uses them as released and is
not affiliated. `rns` and `lxmf` are downloaded hash-locked from PyPI at install time and installed by the user after a
licence notice, never bundled in a 111 release (D26 #5). **Licence of this helper: pending the owner's contact with
upstream** (`LICENSE-NOTE.md`, `LICENSING.md`); until then it ships only behind the same enable gate as every optional
component.

## Files

| File | What |
|---|---|
| `lxmf_sidecar.py` | the program: one file, Python ≥ 3.7 syntax, standard library plus the user-installed `rns` and `lxmf` (imported lazily inside `main`) |
| `requirements.in` | the two exact top-level pins: `rns==1.5.7`, `lxmf==1.2.1` |
| `lxmf-sidecar-lock.txt` | the hash-locked closure (6 packages, 167 hashes: every file PyPI publishes for each pin); `pip install --require-hashes -r` |
| `gen_lock.py` | produces / verifies the lock and writes the same bytes to `10-bootstrap/crates/node111/assets/components/lxmf.lock.txt` (embedded in the signed binary beside `lxmf.toml`) |
| `run-tests.sh`, `tests/` | offline tests (119) with a fake `RNS`/`LXMF` surface; no network, no hardware |
| `LICENSE-NOTE.md` | why there is no licence file yet |

## Run

```sh
python3 -I lxmf_sidecar.py --config-dir <bridge/lxmf/rns> --identity <bridge/lxmf/identity> --storage <bridge/lxmf/store> \
        --name "<display name>" [--announce-interval 60] [-v]
```

`111d` spawns it from `venvs/lxmf` (`HOME=<root>`, piped stdio) only while `rnsd` serves the shared instance (§6 "Start order").
`--announce-interval` is minutes on the monotonic clock (default 60, `0` = events only). `--name` defaults to
`111-<first 4 hex of the address>`. `-v` adds stderr detail (still no foreign text, never key material).

Exit codes: **0** clean (`quit`, stdin closed, SIGTERM/SIGINT) · **1** start-up failure (unreadable identity, Reticulum
failed) · **2** usage (also: a client config that declares interfaces) · **3** partitioned (the helper would have become the
shared instance; the line `{"type":"link","state":"partitioned"}` is written first) · **4** `rns`/`lxmf` not importable (the
stderr line names the venv `<root>/venvs/lxmf`).

## Protocol (JSON Lines, UTF-8, one object per line, ≤ 64 KiB, flushed per line)

`111d → helper` (a malformed line is counted, answered by one stderr line, ignored; it never stops the helper):

| line | notes |
|---|---|
| `{"type":"send","id","to","text","fields"?}` | `id` 1..128 chars; `to` = 32 hex (the peer's `lxmf.delivery` destination hash); `fields` = JSON object, keys are decimal LXMF field ids (`"5"`) and the one name `"relay"` (below) |
| `{"type":"announce"}` | announce now |
| `{"type":"name","display"}` | set the display name (≤ 128 bytes, control characters stripped), then announce |
| `{"type":"quit"}` | clean exit 0 (closing stdin does the same) |

`helper → 111d`:

| line | notes |
|---|---|
| `{"type":"ready","dest","pubkey","instance"}` | always the first line. `dest` = delivery destination hash (32 hex), `pubkey` = the identity's 64 B public key as the library reports it (X25519 ‖ Ed25519, 128 hex), `instance` = the shared-instance name from the client config |
| `{"type":"link","state":"up"\|"down"\|"partitioned"}` | `up`/`down` from the instance link, checked every 5 s; `partitioned` is written once, just before exit 3 |
| `{"type":"msg","hash","src","name","text","claim_ts","signed","why_unsigned","fields"}` | one per delivery; `hash` = the LXMF message hash (111d's dedupe key); `name` = last announced display name or `null`; `claim_ts` = the sender's unverified timestamp; `signed` = the library's `signature_validated`; `why_unsigned` = `source_unknown` \| `signature_invalid` \| `unverified` (else `null`); `truncated: true` / `fields_truncated: true` appear only when something was cut |
| `{"type":"state","id","hash"?,"state","reason"?}` | `hash` is absent until the library has packed the message (the parked row); `reason` only where the table below gives one |
| `{"type":"heard","dest","name"}` | every `lxmf.delivery` announce heard (also feeds Nearby and resumes parked sends); `name` may be `null` |

### Send states (spec §6, v0.3 item 1)

| library | emitted |
|---|---|
| unknown destination (`Identity.recall` finds nothing) | `queued` / `peer_unannounced`, the send is parked (≤ 256) and tried when a matching `heard` arrives |
| `OUTBOUND` (or `GENERATING`) | `queued` |
| `SENDING`, `SENT` | `sent` |
| propagated `SENT` (not used in v1: propagation is off) | `relayed` |
| `DELIVERED` | `delivered` (the recipient's node confirmed) |
| `FAILED` after the helper saw `SENDING`/`SENT` | `sent` / `transmitted, not confirmed` (111d re-queues on `heard`; the same `id` may be sent again) |
| `FAILED` while still `OUTBOUND` | `queued` / `peer_unannounced` |
| `REJECTED` | `failed` / `rejected` |
| user `CANCELLED` | `failed` / `cancelled` |

Monotone: `queued < sent < relayed < delivered`; `delivered` and `failed` are final; a late lower state is ignored; an equal
`queued`/`sent` row is emitted again only to add a new reason. A delivery proof from an older attempt of the same `id` still
counts. The helper never resends by itself: one hand-over to the router per `send`. `SENDING`/`SENT` are not signalled by a
callback in LXMF, so the helper samples in-flight messages once a second, and only while one is in flight.

### Relay marker

`fields.relay = {v, net, id, mid}` on a `send` is carried as `FIELD_CUSTOM_TYPE (0xFB) = "111.relay"` and `FIELD_CUSTOM_DATA
(0xFC) = {v, net, id, mid}` (`v` int 0..255, `net` ≤ 64 B, `id`, `mid` ≤ 256 B, all required; a malformed marker makes the
line malformed). A received message lists every integer field id (`"251"`, `"252"` for the marker) and adds `fields.relay` when
both custom fields are present and well formed (a foreign sender cannot set the `relay` key itself: only integer ids are read).
Stock apps ignore custom fields, hence the visible `[name/TAG] ` prefix in `text` (§8).

### Received fields and attachments

Attachments (`FIELD_FILE_ATTACHMENTS` 5, `FIELD_IMAGE` 6, `FIELD_AUDIO` 7) are not forwarded: the field becomes
`{"attachment": {"bytes": N}}`. Tickets (`0x0C`) are dropped. Other values are bounded: bytes become hex cut to 256 bytes,
strings 1024 chars, 32 entries per container, depth 5, 16 KiB of rendered fields in all. The text is cut to 64 KiB and, when
JSON escaping would push the line past 64 KiB, cut further (`truncated: true`): a line never exceeds the protocol limit.

## What it does at start (spec §6 (1)–(3))

1. Refreshes the client config in `--config-dir` (dir 0700, `config` 0600): `share_instance = Yes`, `instance_name` (default
   `111`). `111d` renders this file with the `rpc_key`; the helper never generates one (a missing config is written without
   it, with a stderr warning) and refuses a config that declares interfaces.
2. `RNS.Reticulum(configdir, require_shared_instance=True)`. **Verified against rns 1.5.7** that this mode exists: it detaches
   a would-be instance and raises, so the helper exits 3. A RNS without the parameter falls back to the
   `is_connected_to_shared_instance` flag. The start order (rnsd first) and the post-spawn `rnstatus` check stay the
   mechanism of record (§6); this is the second line of defence.
3. Loads or creates `--identity` (0600; an existing file that cannot be read is never replaced), one `LXMRouter(storagepath)` and
   one delivery destination, no stamp cost advertised, then writes `ready`, `link`, and announces. Announces: on start, on every
   link up, on `announce`/`name`, and every `--announce-interval` minutes after the last announce.

Reticulum's own log goes to stderr through a callback (ASCII, bounded, warnings and up); `fd 1` is pointed at stderr after
start so nothing a library prints can corrupt the protocol.

## Tests

`sh run-tests.sh` (pytest when the interpreter has it, else unittest; `-I` isolated mode). They cover framing and the 64 KiB
limit, every command and malformed-line case, the state mapping (all library states × every prior state), the monotone
rule, parking and resume, the marker placement and round trip, attachment stripping and caps, announce timing on a fake
monotonic clock, exit codes 3 and 4, a subprocess run of the real `__main__` path (library output never reaches the protocol,
SIGTERM is a clean exit), no foreign text or key material on stderr, and AST scans: no socket, no `bind`/`listen`, no wall
clock, no `print`. `tests/test_lock.py` checks the lock and the embedded copy. The fake surface lives in `tests/_support.py`.

Besides the offline tests, the helper was run against real `rns 1.5.7` / `lxmf 1.2.1` from a venv built from the lock: two
`rnsd` instances (TCP server and client on loopback), one helper on each: attach as client, `ready`, announces both ways
(`heard` with names), a relayed message with the marker (`signed: true`, marker intact, same hash on both ends), `delivered`,
a 3000-byte message (link/resource path), `queued`/`peer_unannounced` for an unannounced address, and a send to a dead peer
ending `queued → sent → sent / transmitted, not confirmed` after 63 s. That run is not a repo test (G3 owns it, §16).

## Weight (spec §2.1 rule 5; measured 2026-10-19, x86_64, CPython 3.13)

Resolved: `rns 1.5.7` · `lxmf 1.2.1` · `cryptography 50.0.2` · `cffi 2.1.1` · `pycparser 3.11` · `pyserial 3.5` (resolved on
CPython 3.11, so the lock's floor is **CPython 3.11**: `cffi 2.1.1` and `pycparser 3.11` need 3.10, and `cryptography 50` needs `typing-extensions` below 3.11, which this lock (resolved on 3.11) does not carry — on 3.10 `pip --require-hashes` would refuse it; although `rns` and `lxmf`
need only 3.7).
Cached payload: 5.83 MB of wheels (this platform; 167 hashes cover every published file). Installed: 24.9 MB (site-packages
without pip; `cryptography` is 15.5 MB of it). Idle helper: 43.7 MB RSS, 10 threads, **no TCP/UDP/unix listener**. Zero while
the component is off (no venv, no process).

**`rns` pairing.** `lxmf 1.2.1` requires `rns>=1.5.7`, so this lock pins `rns==1.5.7` (the earlier scaffold pinned 1.5.6, which
cannot coexist with `lxmf 1.2.1`). The Layer 1 `rns-lock.txt` must move to the same `rns` version at release time (two `rns`
copies share one instance protocol, §6); `gen-notices.py --check` or the release checklist asserts it.

## Regenerating the lock

```sh
python3.11 -m venv /tmp/lockvenv                              # the oldest CPython available
python3 -I gen_lock.py --pip "/tmp/lockvenv/bin/python -m pip"   # needs network; writes both copies
python3 -I gen_lock.py --verify                                # offline structural check, both copies identical
```
