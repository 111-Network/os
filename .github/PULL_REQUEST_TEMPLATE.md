<!-- Keep it short. The board row and the diff are the record; this is the index to them. -->

## What and why

Layer / task row: `N-<slug>/TASKS.md` row …  (or: no row — say why)

## Verified how

Gate(s) run and their last line (for example `1-bootstrap/scripts/check.sh` → `GREEN (G2 SKIPPED — not a lock run)`):

```
```

## Record kept

- [ ] the task row carries the as-built facts (or n/a)
- [ ] `plan/DECISIONS.md` has an entry if a decision was made or changed
- [ ] `LICENSING.md` / `THIRD-PARTY-NOTICES.md` updated if a dependency, vendored file or pinned program changed
- [ ] the Raspberry Pi floor holds: no resident process, poll, timer, runtime dependency or build-time weight added to the required configuration (or the owner said yes, and it is measured in the layer's `TASKS.md`)
- [ ] no attribution lines in the commits; no model names or identifiers in files
