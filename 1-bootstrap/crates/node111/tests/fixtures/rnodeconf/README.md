# rnodeconf `--info` goldens (Layer 6 §12, identify step 1)

Text `rnodeconf --info <port>` prints, written from the output statements of
`RNS/Utilities/rnodeconf.py` in the `rns` 1.5.7 wheel (PyPI; compact log format
`[YYYY-MM-DD HH:MM:SS] <msg>`, tab-indented `Device info:` fields), **not**
captured from a board: the parser (`daemon::devices::identify::rnode::parse_info`)
is *unverified on hardware* until external gate G1 runs. No stub here is modelled
on Reticulum source (R06 G6): `tests/fake_rnodeconf` only prints these files.

| File | rnodeconf path | Outcome |
|---|---|---|
| `info-provisioned.txt` | detect, EEPROM provisioned, host-controlled | `Match` rnode 1.86 |
| `info-tnc.txt` | detect, EEPROM provisioned, TNC mode | `Match` rnode 1.85 |
| `eeprom-invalid.txt` | detect, EEPROM blank | `Match` rnode 1.74 (the firmware answered) |
| `no-rnode.txt` | `device_probe()` failed | `NoMatch` |
| `cannot-open.txt` | the port did not open | `Error` |
| `garbage.txt` | the tool is broken | `Error` |
