# lxmf-sidecar

The Layer 6 LXMF helper (spec `os/60-transport/README.md` §6, D26 #4): the one program of ours that imports Reticulum. It
attaches to the node's shared `rnsd` instance as an interface-less client, owns one LXMF delivery identity, and speaks JSON
Lines over piped stdio to `111d` (stdout is the protocol, stderr the log). It never listens on a port.

**Credit.** Reticulum and LXMF are the work of the Reticulum project (reticulum.network); 111 uses them as released and is
not affiliated. `rns` and `lxmf` are downloaded hash-locked from PyPI at install time and installed by the user after a
licence notice — never bundled in a 111 release (D26 #5). **Licence of this helper: pending the owner's contact with
upstream** (`LICENSING.md`); until then it ships behind the same enable gate as every optional component.

T0 scaffold (`TASKS.md`): layout only. Wave 2 C3 writes `lxmf_sidecar.py`, the lock (`gen_lock.py`), the offline tests
and the component manifest `assets/components/lxmf.toml`.

## Files

| File | What |
|---|---|
| `lxmf_sidecar.py` | the program (one file; Python ≥ 3.7, standard library plus the user-installed `rns` and `lxmf`) — C3 |
| `requirements.in` | the top-level pins; `rns` must resolve to the same version as `rns-lock.txt` at release time |
| `lxmf-sidecar-lock.txt` | the hash-locked closure (`pip install --require-hashes -r`) — C3 |
| `run-tests.sh`, `tests/` | offline tests with a fake `RNS`/`LXMF` surface; no network, no hardware |

## Protocol (JSON Lines, UTF-8, one object per line, ≤ 64 KiB)

- `111d → helper`: `send {id, to, text, fields?}` · `announce {}` · `name {display}` · `quit {}`
- `helper → 111d`: `ready {dest, pubkey, instance}` · `msg {hash, src, name, text, claim_ts, signed, why_unsigned, fields}` ·
  `state {id, hash, state, reason}` · `heard {dest, name}` · `link {up | down | partitioned}`

## Run

```sh
python3 -I lxmf_sidecar.py --config-dir <bridge/lxmf/rns> --identity <bridge/lxmf/identity> --storage <bridge/lxmf/store> --name <display name>
```
