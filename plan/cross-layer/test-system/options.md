# Cross-layer test system: options brief (D36)

Status: drafted 2026-11-11 at `271dabd`; the owner's "go with defaults" (D36 item 3) applies the recommendations unless the lead
reverses one. Written from the five survey reports in `research/` and the seam × evidence matrix [`matrix.md`](matrix.md); the facts
below were re-read in the sources (nothing built or run). Reads with: D36 · `plan/cross-layer/ci-cd.md` (D28, the parity rule) ·
`AGENTS.md` (the Pi floor, the box rules) · `plan/EXECUTION.md`.

Owner request (D36, verbatim): "I want you to connect layers and expand or fix or develop if none existing: a solid test system for
this code base that helps AI to debug the connection between layers etc." The repo rule: adopt and extend, never reinvent. So this
brief builds **one entry point on top of the gates that exist**; it replaces no harness.

## 0. What the survey settled (facts that bound every option)

1. **There is no single entry point.** The nearest is `1-bootstrap/scripts/check.sh` (POSIX sh, `set -eu`): fmt, clippy, node111
   `cargo test`, shellcheck, four hermetic gate tests, notices, systemd-analyze, then the nested `5-messaging/scripts/check.sh`, G2,
   the presence sidecar and the UI `pnpm check`. It stops at the first failure and names only the last `== stage ==` header.
   `3-intelligence/scripts/check.sh`, Layer 2's CI steps, `build.py --check`, `local-smoke.sh` and every `verify/` harness are separate.
2. **The seams: 89 exist, 14 are planned** (matrix count correction: 05 dropped the 3 `X:` rows). Almost every one goes through one
   binary, `111d`: path deps on `id111` and `mesh111`, the embedded UI, and three kinds of supervised child. `agent111` has no path
   dependency at all; its seams are env vars, files and loopback HTTP.
3. **Running-system evidence exists for 42 of 89 seams, all of it local.** It comes from 8 namespace harnesses: S1/S2 `smoke.sh`,
   `panel-lint.sh`, G1 `bridge.sh`, G3 `lxmf.sh`, G4 `spine.sh`, G5 `devices.sh`, G7 `zero-weight.sh` and `linux-e2e.sh --namespace`.
   No aggregate gate and no CI job runs any of them, because the hosted runners restrict `unshare -Urnm`.
4. **The harnesses already share one output idiom.** Checks print `ok: <desc>` / `FAIL: <desc>  [HTTP st: body]` / `note:`, with
   `== N. phase` headers. The closing line is `<GATE> GREEN` / `<GATE>: N FAIL`, plus `<GATE> SKIPPED: <reason>` with exit 0
   unless `REQUIRE_<GATE>=1` (`6-transport/verify/lib.sh`). `panel-lint`'s browser driver alone prints stable rule ids
   (`<scope>/<rule-id>`). Nothing anywhere writes machine-readable results.
5. **Exit codes lie in four places.** (a) `smoke.sh`/`panel-lint.sh`: a `set -e` abort in the namespace still lets the EXIT trap
   write `FAILS=0`, so the outer prints GREEN (`smoke.sh:1653-1657`). (b) `skip_or_fail` exits 0 SKIPPED after `FAIL:` lines
   (`spine.sh:557`, `devices.sh`, `bridge.sh:1097`). (c) `local-smoke.sh` counts `linux-e2e`'s exit 77 (SKIP) as FAIL and overwrites
   one shared log per gate. (d) `check.sh` and the mesh111 gate exit 0 on every skip; only the last line tells.
6. **Fakes stand in on 13 seams and are checked against nothing they mirror.** The three agent111 stub daemons are hand-typed
   JSON. The other unchecked fakes: node111's stub agents, `fake_lxmf_sidecar.py`, `fake_deltachat_rpc.py`, two `stub-meshmud.py`
   plus `fake-board.py`, `stub-ntfy.py`, `stub-llama.py`.
7. **One contract chain is real and two-sided.** `openapi.yaml` → `gen-sse.mjs` → `ui/test/contract.gen.ts`, read by the UI tests
   and by five node111 test files. Freshness is checked only by vitest and the CI diff, so a Rust-only run trusts a stale digest.
   No check compares the daemon's routes with the yaml (`dashboard_auth.rs` `UNROUTED` lists 9 contract-only operations).
8. **Recorded traffic is the best daemon → UI evidence, and half of it is unread.** 11 tapes from S1/S2 are replayed by
   `tapes.test.ts`. G1's two Layer 6 tapes have no reader. `7-relay/verify/tapes/` is empty.
9. **The box rules are prose today.** They are: at most two cargo builds at once; never `smoke.sh` with `panel-lint.sh`; nothing
   left running; scratch outside the repo. The harnesses use fixed ports (11180, 11183, 11230, 11280) and `/tmp`.
   `carrier.sh --smoke` writes into the tracked tree (`7-relay/verify/logs/`).
10. **Root changes results.** As euid 0, 8 node111 tests fail by design (`6-transport/TASKS.md:80`). `api_v1_devices.rs` returns early
    and counts PASS. Permission faults (`0o500`) become invisible.
11. **Cost.** A namespace harness takes 3–5 min. The node111 suite is 446 test fns in 35 binaries over a 14 GB target dir. The mesh111
    `forward` suite takes ~3 min. `python3` ≥ 3.11 is already required (`build.py`, the Layer 6 harnesses, both sidecars).
12. **Layer 7 verify is mostly T0 stubs** (G7-ext, G8, G9, G10 default, G11, G12 print `SKIPPED (T0 stub)`). Layers 8 and 9 have
    no code and no spec folder yet (D34/D35: specs in drafting).

## 1. What the test system is, in one paragraph

One command, `python3 verify/run.py`, runs any slice of the existing gates and harnesses: by layer, by seam id, by check id or by
cost tier. It enforces the box rules itself. It turns whatever each gate prints into one JSON line per result. Each line carries
a stable id, the seam and the layers it joins, PASS/FAIL/SKIPPED with a reason, the failing assertion's own words, the log and
line, a one-check re-run command, the spec section and the source files on each side. The run ends with a short human summary.
The seam map is data (`verify/seams.toml`, `verify/checks.toml`), and a lint check holds it to the tree. The map also names
every fake and what it mirrors, so a stub's distance from reality shows up as a counted gap. The system lives in the repo only.
It adds nothing to an installed node.

## 2. Forks (the lead decides; recommendation first)

### F1. The runner's language

- **A. Python 3.11, stdlib only, run as `python3 -I`.** JSON, TOML (`tomllib`), subprocess sessions, timeouts and `fcntl.flock`
  slots are built in, and unit tests use `unittest` with no new tool. Python ≥ 3.11 is already a hard prerequisite of `build.py`,
  every Layer 6 harness and both sidecars. Cost: a second language beside the sh gates, but only for the driver; every gate stays
  as it is. *(pick)*
- B. POSIX sh like the gates. No new language, but JSON escaping, TOML parsing, per-check timeouts, two-slot locks and process-group
  cleanup would all be hand-rolled. The scouts already flag 1,000+-line sh harnesses as walls an agent cannot read.
- C. A Rust `xtask`. Typed, but it needs a cargo build before any check (one of the two cargo slots, minutes cold). It also breaks
  exactly when the tree does not compile, which is when an agent needs it most.

### F2. Where it lives

- **A. A root folder `verify/`** beside `hooks/`: `run.py`, `seams.toml`, `checks.toml`, `t111/` (the package), `lib/result.sh`,
  `tests/`, `README.md`. It is cross-layer code, so it fits no `N-<slug>` folder, and it mirrors each layer's own `verify/`.
  `AGENTS.md` "Read first" gains one line. *(pick)*
- B. `plan/cross-layer/test-system/`. Keeps it in the plan, but `plan/` holds plans and run journals, not gates CI calls.
- C. `1-bootstrap/verify/` next to `local-smoke.sh`. That makes Layer 1 the owner of seams it is only one side of.

### F3. How existing harness output becomes records

- **C. Parse first, then one shared emitter.** Phase 1: the runner parses what the gates print today (sec. 3.4). The
  `ok:/FAIL:/SKIPPED` idiom, the libtest lines and vitest's JSON reporter are regular enough, and the runner catches the lying
  exits of fact 5 itself. Phase 2: one `verify/lib/result.sh` is sourced by the five helper definitions of `ok`/`fail`/`skip`
  (`smoke.sh`, `panel-lint.sh`, `6-transport/verify/lib.sh`, `spine/node.sh`, and the L7 lib through the L6 one). When
  `T111_RESULTS` is set, it also appends one JSON line per call; when it is unset, the text output stays byte-identical. Ids are
  `<check>/<phase>/<slug of the description>`; an explicit `@id` is added only where a seam needs one. Cost: 5 helper functions
  edited, no assertion touched. *(pick)*
- A. Parse only. No harness edit, but stdout/stderr ordering and multi-line FAIL bodies stay heuristics forever.
- B. Teach every harness the emitter now and give every assertion an id. That is ~350 `t` call sites in six green harnesses that
  belong to locked layers: churn and review load with no new evidence.

### F4. The seam map's format

- **A. TOML** (`tomllib` is stdlib, comments allowed, diffs read well). *(pick)*
- B. JSON (the UI could read it natively, but it allows no comments, and the map needs its reasons inline).
- C. YAML (needs PyYAML in the runner; `build.py` already needs it, but the runner should not).

### F5. How Layers 8 and 9 plug in

- **A. Their 14 seams enter the map now as `planned` rows** (ids from 05). Each spec's seam section uses those ids. Each layer's
  board carries a verify row that registers its gate script in `checks.toml` and promotes its seams to `built` with a check or a
  declared `gap`. The lint warns when a planned seam's side files appear in the tree (code landed, map not promoted). Layer 9's
  on-node observer neither imports nor ships any of this; at most it may borrow the seam ids as words in a diagnosis. *(pick)*
- B. Per-layer seam files (`<layer>/verify/seams.toml`) merged at run time: fewer edit collisions between parallel agents, but no
  longer "one file an agent can read".
- C. Wait until code lands. That loses the ids the specs would cite.

### F6. CI scope

- **A. One new job `cross-layer`: `python3 -I verify/run.py --tier fast --ci --scratch "$RUNNER_TEMP/verify"`**, which uploads
  `results.jsonl` as an artifact. The four existing jobs stay unchanged. After two green weeks, `bootstrap-gate` becomes
  `run.py --layer 1 --gate` (it still runs `check.sh` underneath, so the parity rule of `ci-cd.md` holds) and emits records too.
  The `ns` tier stays local; the lead runs it before a layer lock (the Layer 7 V9-style row). *(pick)*
- B. Move every job onto the runner at once. One change, but it puts the runner's first bugs on every PR.
- C. No CI change. That leaves the fast seam checks (map lint, digest freshness, cross-implementation vectors) off every PR.

### F7. Root and known failures

- **A. A `[[known]]` table in `checks.toml`** (test id, condition `euid0`, source `6-transport/TASKS.md:80`). When the condition
  holds and the test fails, the record is SKIPPED with reason `known: fails as root (…)`; when it passes, it counts PASS.
  `--require` turns it back into FAIL. The `api_v1_devices.rs` root early return goes in the same table, so it reports SKIPPED,
  not PASS. *(pick)*
- B. Fix the 8 tests now (Rust, separate rows; worth doing later regardless).
- C. Leave them red. Every root agent then re-learns the board sentence.

### F8. Re-running one assertion of a namespace harness

- **A. For now the re-run command is the whole harness with its keep switch, and the record states the cost** (`rerun_cost: "ns,
  ~4 min"`). Cargo, vitest and pytest records get a single-test command (seconds). *(pick)*
- B. Add `ONLY=<phase,…>` to `smoke.sh` and the L6 gates (setup phases always run). This is a later row, once the results show which
  harness fails most.

## 3. Detail picks (the design the board builds)

### 3.1 The entry point and selection

```
python3 -I verify/run.py --scratch DIR [--layer 3,4] [--seam L3-L4:chat-relay] [--check l4.smoke] [--tier fast,slow,ns]
                         [--gate] [--list] [--lint] [--matrix] [--require] [--keep] [--cargo-slots 1|2] [--offline] [--ci]
```
- `--scratch` is required: every byte the run writes lands under `DIR/<run-id>/`, where run-id is UTC time plus the short HEAD.
- Tiers: `fast` (seconds, no cargo/pnpm build, no namespace), `slow` (a cargo or pnpm build or suite: minutes), `ns` (needs
  unprivileged user/net/mount namespaces or root: minutes, one at a time). The default is `fast,slow`; `ns` is opt-in.
- Two kinds of catalog entry. **Gates** run a layer's own script as is (`check.sh`, `3-intelligence/scripts/check.sh`, the id111
  cargo trio, `5-messaging/scripts/check.sh`, `pnpm check`, `build.py --check`, each `verify/*.sh`). **Units** run one test binary,
  one vitest file or one harness alone, mapped to seams. `--gate` and full runs execute gates and derive unit records from the
  gate's output (no suite runs twice). `--seam`/`--check` execute only the units that cover the selection, the fastest way to
  reproduce. A unit names the gate that covers it (`covered_by`).
- `--list` prints the plan (checks, tiers, locks, est. time) and runs nothing. `--lint` runs only the map lint.
  `--matrix` prints `matrix.md`'s tables from the map plus the last results, so the matrix stops being hand-kept prose.

### 3.2 The box rules, built in

- **Cargo**: every check with `cargo = true` (any gate or harness that builds) takes one of `--cargo-slots` slot locks (default 1,
  max 2) via `flock` on files in `${T111_LOCKS:-/tmp/111-verify-locks}`, so two runner instances share the box's limit of two.
  `CARGO_TARGET_DIR` is passed through; with none set, each layer's default target is used (a private one is the caller's choice,
  under their scratchpad).
- **One verify harness at a time**: every `ns` check takes the exclusive lock `verify`. This also enforces "never `smoke.sh` and
  `panel-lint.sh` together".
- **Ports**: before an `ns` check the runner reads `/proc/net/tcp{,6}`. If 11180, 11183, 11230 or 11280 is held, the check is
  SKIPPED with reason `port 11180 held by pid N (<cmdline>)` instead of colliding.
- **Nothing left running**: each check runs in its own session (`setsid`). A timeout sends TERM to the group, then KILL after 10 s.
  After each check the runner compares listeners and processes against the pre-run snapshot. It looks for anything whose cmdline
  or environ names the scratch dir, or whose binary is ours (`111`, `111-agent`, `stub-*.py`, rnsd, meshmud, the headless browser,
  vite). A leftover becomes a `hygiene/<check>` FAIL naming the pid and cmdline, and is then killed.
- **Scratch and tree**: `TMPDIR=DIR/<run-id>/<check>/tmp` (every harness `mktemp` lands inside); `out.log` per check. A green
  check's scratch is deleted unless `--keep`; a red one is kept and named in its record. If `git status --porcelain` changes during
  a check, that is a hygiene FAIL naming the files (it catches `carrier.sh`'s log and a stray `smoke-probe-*.txt`).
- `--offline` marks the checks that need the package index (G3, G4, G5) SKIPPED `offline` up front, instead of letting them skip
  silently mid-run.

### 3.3 The result record and the summary

`DIR/<run-id>/results.jsonl` starts with one `run` record (head, dirty flag, euid, tools found, namespaces usable: so every skip
reason is checkable). Then comes one `check` record per result (illustrative):
```json
{"v":1,"kind":"check","id":"l1.node111/api_v1_relay/<test fn>","check":"l1.gate","seams":["L3-L4:chat-relay"],
 "layers":[3,4],"status":"FAIL","reason":null,"assertion":"<the panic or FAIL line, verbatim, ≤ 2 KB>",
 "log":"DIR/<run-id>/l1.gate/out.log","log_line":5121,
 "rerun":"cargo test --manifest-path 1-bootstrap/Cargo.toml -p node111 --test api_v1_relay -- --exact <test fn>","rerun_cost":"slow, ~20 s warm",
 "spec":["4-interface/README.md §10","4-interface/api/README.md"],
 "sides":{"a":{"layer":4,"files":["1-bootstrap/crates/node111/src/daemon/dashboard/api_v1/relay.rs"]},
          "b":{"layer":3,"files":["3-intelligence/crates/agent111/src/http.rs","3-intelligence/crates/agent111/src/chat.rs"]}},
 "read_first":"a","fake":{"path":"1-bootstrap/crates/node111/tests/api_v1_relay.rs","mirrors":"b"},"strength":"F","tier":"slow","duration_s":0.4}
```
`read_first` follows one rule. A check against a fake points at the side under test; the record names the fake and what it mirrors.
A contract-schema failure points at the producer of the payload. A running-system failure points at the seam's default
(`read_first` in the map: the producer of the data the assertion reads). `status` is only PASS, FAIL or SKIPPED; a SKIPPED record
always has a `reason`. The human summary (`summary.txt`, also stdout) gives one line of counts, then one block per FAIL, then one
line per SKIPPED, then the last line:
```
verify: 1214 PASS, 2 FAIL, 9 SKIPPED — 14 checks, fast+slow, 6m12s — DIR/<run-id>/results.jsonl
FAIL L3-L4:chat-relay [L3↔L4] l1.node111/api_v1_relay/<test fn>: <assertion, first line>
     read first: …/api_v1/relay.rs (a, under test; b is a fake) · spec 4-interface/README.md §10
     rerun: cargo test … -- --exact <test fn> · log: …/l1.gate/out.log:5121
SKIPPED L2-L7:epoch-linkkey l7.g11: T0 stub (7-relay/TASKS.md V5)
VERIFY RED (2 FAIL)
```
The last line is `VERIFY GREEN`, `VERIFY GREEN (n SKIPPED — not a full run)` or `VERIFY RED (n FAIL)`, with exit 0, 0, 1. Under
`--require`, any SKIPPED is a FAIL. A usage error exits 2.

### 3.4 Adapters (what each gate's output becomes)

- **libtest**: `Running <path> (…)` names the binary. `test <name> ... ok|FAILED|ignored` gives one record per test fn. The
  `---- <name> stdout ----` block gives the assertion (the `panicked at` line and its message) and the log line.
- **vitest**: units run with `--reporter=default --reporter=json --outputFile=<scratch>/vitest.json`; the gate (`pnpm check`) is
  parsed from `FAIL test/<file> > <describe> > <name>` lines. **pytest/unittest**: run with `-rs` so skips are visible;
  `FAILED <file>::<name> - <msg>` / `FAIL: test_x (module.Class)`.
- **okfail** (every sh harness): stdout and stderr go into one ordered stream (`2>&1`). `ok:` → PASS and `FAIL:` → FAIL (the text up
  to `  [HTTP` is the assertion, the bracket is kept). `<G> SKIPPED: <reason>` → SKIPPED; `note: scratch kept at <dir>` → the
  record's log dir. `== <phase>` → the phase, mapped to seams by the check's `phases` table. The runner then checks the outcome:
  exit 0 with any `FAIL:` line = FAIL (fact 5b). A missing closing `GREEN` line, or the inner run's last phase never closed = FAIL
  `harness did not finish (last phase: <name>)` (fact 5a), before the harness fix lands. A cascade is marked: a FAIL in a phase
  after an earlier FAIL in the same check gets `"after":"<first FAIL id>"`, so an agent reads the first one first.
- **stagewise** (`check.sh` and the mesh111 gate): `== name ==` headers split the stream, and each stage goes to its inner adapter
  (`test (host)` → libtest, `ui` → vitest text, sidecar → pytest). A `… SKIPPED` line → a SKIPPED record for that stage. A
  non-zero exit → FAIL for the open stage, with its last 40 lines. Stages after it → SKIPPED `not reached (check.sh stopped at
  <stage>)` until the keep-going switch lands (X8).
- **table** for the Monocypher harness (`FILE … RESULT` rows, `FAIL [<file>] <msg>`); **buildpy** for `ERROR:<line>  <msg>` (file
  and line become the record's evidence).
- **emitter**: when a check's `T111_RESULTS` file holds lines (F3 phase 2), they replace the parse for that check. Text parsing
  stays as the fallback.

### 3.5 The seam map as data

`verify/seams.toml`, one table per seam, sorted by id:
```toml
[[seam]]
id = "L3-L4:chat-relay"
status = "built"                     # built | stub | planned
spec = ["4-interface/README.md §10", "4-interface/api/README.md"]
a = { layer = 4, files = ["1-bootstrap/crates/node111/src/daemon/dashboard/api_v1/relay.rs"] }
b = { layer = 3, files = ["3-intelligence/crates/agent111/src/http.rs", "3-intelligence/crates/agent111/src/chat.rs"] }
read_first = "a"
symptom = "UI error banner with a problem code; the stream ends without done"
gap = ""                             # required text when built and no check of strength R
```
`verify/checks.toml` holds the `[[check]]` entries. Each has id, kind (gate/unit), cmd, cwd, tier, layers, seams, `phases` (phase
prefix → seams), adapter, `cargo`, `lock`, timeout, env, the rerun template, `covered_by` and strength. Alongside them sit
`[[fake]]` (path, mirrors, seam, parity check or `"none"`), `[[known]]` (F7) and `[unmapped]` (test or harness files on purpose
not tied to a seam, each with a reason).

**The lint** (`run.py --lint`, check `x.seams.lint`, tier fast, in CI) fails when:
1. an id is duplicated or does not match its layers;
2. a side file, a check's script or test file, or a spec file is missing, or a `§N.M` reference matches no heading (`^#+ N\.M`);
   the numberless Layer 1 and 3 sections are matched by heading text;
3. a check names an unknown seam;
4. a `*/verify/*.sh`, a `tests/*.rs` of node111/mesh111/id111/agent111, a `ui/test/*.test.ts` or a sidecar `run-tests.sh` is
   claimed by no check and absent from `[unmapped]`;
5. a `built` seam has no check and no `gap`;
6. a `[[fake]]` names a missing mirror;
7. any path under `verify/` is named by `1-bootstrap/manifest/` or `cross-build.sh` (the Pi floor).

It warns when a `stub`/`planned` seam's side file has lost its T0 marker (promote the row). It also warns with the count of fakes
with `parity = "none"`, which the summary repeats on every run.

### 3.6 The day-one catalog (all existing gates; no new assertion needed)

- **fast**: `x.seams.lint` · `l0.kb.check` (`build.py --check`) · `l0.kb.selftest` · `l4.contract.fresh`
  (`gen-sse.mjs --check`; every Rust unit that reads `contract.gen.ts` `requires` it, which fixes fact 7's stale-digest read) ·
  `l2.vectors.crosscheck`, `l5.vectors.crosscheck` · `l2.monocypher` · `x.notices` · the five hermetic gate tests of
  `1-bootstrap/scripts/tests/` and `notices/tests/build-scripts-gate.sh` · `l1.shim`, `l1.termux` · `l5.presence-sidecar` ·
  `l6.lxmf-sidecar`.
- **slow**: `l1.gate` (`check.sh`) with units per node111 test binary · `l2.gate` (the id111 cargo trio) · `l3.gate` · `l5.gate`
  with units per mesh111 test file · `l4.ui` (`pnpm check`) with units per test file · `l7.g10-smoke` (`carrier.sh --smoke`).
- **ns**: `l4.smoke`, `l4.smoke3`, `l4.panel-lint`, `l6.g1`, `l6.g3`, `l6.g4`, `l6.g5`, `l1.linux-e2e`, and `l7.g7ext` … `l7.g12`
  (SKIPPED `T0 stub` until their V rows land; the record carries the board row).

## 4. The new seam checks worth adding first (from the matrix gaps)

Ordered by evidence gained per hour. Each is a board row (sec. 6):
- **N1 Contract routes, both directions.** Extend `dashboard_auth.rs`'s walk of every contract operation from writes to every
  method. Assert each operation not in `UNROUTED` reaches a handler (not the router's fallback), and the runner orders it after
  `l4.contract.fresh`. *Closes:* `L1-L4:openapi-rust` (B/D → A).
- **N2 Agent stub-daemon bodies become shared fixtures.** The three agent111 stubs load
  `3-intelligence/crates/agent111/tests/fixtures/daemon/*.json` (`include_str!`). A new `ui/test/agent-fixtures.test.ts`
  validates each body against its operation's response schema, and each proposal body against `Proposal`, kinds included.
  *Closes:* `L3-L5:daemon-tools`, `L3-L6:l6-tools`, the kind pin of `L3-L4:proposals-fs`.
- **N3 The Layer 6 tapes get their reader.** `tapes.test.ts` also reads `6-transport/verify/tapes/` (and `7-relay/verify/tapes/`
  when it fills). *Closes:* the unread recordings; L6 frames from a real daemon.
- **N4 G7 inside S1.** `smoke.sh` runs `zero-weight.sh` against its own daemon before 5h. *Closes:* `L1-X:pi-floor` (D → A, every
  smoke run).
- **N5 The independent legs run.** Both `crosscheck.py`, the Monocypher harness and `build.py --selftest` (catalog only, plus CI
  through F6). *Closes:* D for `L2-L5:test-vectors`.
- **N6 LXMF fake parity.** One protocol transcript `6-transport/sidecars/lxmf-sidecar/tests/fixtures/protocol.jsonl` is replayed
  against the real helper (on its fake RNS) and against `fake_lxmf_sidecar.py`; the reply lines must match (ids normalised).
  *Closes:* the fake gap on `L6-L6:lxmf-sidecar`.
- **N7 The presence file schema.** `5-messaging/sidecars/meshtastic-presence/presence.schema.json`. The real reader's tests, a
  `--selfcheck` of both `stub-meshmud.py --role presence`, and a `presence.rs` unit test validate against it. *Closes:*
  `L5-L6:presence-file`.
- **N8 Smoke's payload is the real one.** `make_root` builds `payload.manifest` with `gen-manifest.py`. *Closes:* the hand copy on
  `L0-L1:kb-payload`.
- **N9 A real agent calls a Layer 6 tool.** G1's stand-in agent becomes the real `111-agent` plus `stub-llama.py`
  (`@tool bridge.read`, `@tool bridge.propose_edge`); accept → the edge row exists. *Closes:* `L3-L6:l6-tools` (F → R).
- **N10 Watchdog.** A node111 unit test on a `NOTIFY_SOCKET` datagram socket: `WATCHDOG=1` arrives within 15 s.
  `systemd-analyze` stops being `|| true` where the tool exists. *Closes:* `L1-L1:watchdog` (N → I).
- **N11 Prompts name real tools.** `chat_loop.rs` writes and checks `tests/fixtures/tools.json`; `ui:prompts` checks every tool
  named in `prompts.ts` against it. *Closes:* `L3-L4:prompts-tools`, the tool-count pin in three places.

Later: the deltachat fake against a recorded real session, `stub-ntfy.py` against an owner-run ntfy (G10's items), the 8 root
failures (F7 B), the `ONLY=` filter (F8 B), one shared contract validator in place of the five Rust ports.

## 5. CI, local, and the Pi floor

- **CI** (F6 A): the `fast` tier and the lint, on every PR; `results.jsonl` is uploaded so a reviewing agent reads records, not a
  log. `slow` stays in the four existing jobs until the switch. Shellcheck widens to `3-intelligence/scripts`, `4-interface/verify`,
  `6-transport/verify`, `7-relay/verify`, `verify/` and `hooks/` (the existing `check.sh` stage, more paths).
- **Local** (the lead, and any agent on a box with namespaces): `run.py --tier fast,slow` before each commit that touches a seam's
  side files. The runner prints that set (`--seam` from the changed paths: `run.py --changed` maps `git diff --name-only` to seams
  through the side files). `run.py --tier ns --require` runs before a layer lock and at handoff.
- **The Pi floor**: the test system lives in the repo only. `verify/` is never in the payload (lint item 7). It is stdlib Python
  with no pip install, no service, no timer, and nothing in `111`. It does not reach the required configuration at all. What it
  adds to the floor is measurement: G7 then runs on every smoke run (N4), and its RSS/listener notes go into records so a change
  shows as a diff between runs.

## 6. Task list (a board can be made from this)

| id | task | files owned | check | class |
|---|---|---|---|---|
| X1 | Seam map and catalog from `matrix.md` (89 + 14 rows, every check of 3.6, fakes, known, unmapped) | `verify/seams.toml`, `verify/checks.toml` | `run.py --lint` green; every matrix id present | shell/Python/docs |
| X2 | Runner core: selection, tiers, locks, sessions, timeouts, hygiene, scratch, records, summary, exit codes | `verify/run.py`, `verify/t111/{select,run,locks,hygiene,record,summary}.py` | `python3 -I -m unittest discover -s verify/tests`: fake checks that pass, fail, skip, hang (killed), leak a process (caught), dirty the tree (caught); never > 2 cargo slots; `verify` lock serial; nothing outside `--scratch` | Python |
| X3 | Adapters of 3.4 | `verify/t111/adapters.py`, `verify/tests/fixtures/<adapter>/*.txt` + `.jsonl` | golden JSONL per captured output, including smoke's silent-abort shape and a skip-after-FAIL | Python |
| X4 | Lint of 3.5, `--matrix`, `--changed` | `verify/t111/{lint,matrix,changed}.py` | unittest on mutated maps; `--matrix` reproduces `matrix.md`'s tables | Python |
| X5 | Harness honesty: (a) `smoke.sh`/`panel-lint.sh` ERR trap prints `FAIL: aborted in <phase> at line N` and the result file says `DONE=1` only at the end; (b) `skip_or_fail` exits 1 once `FAILS>0`; (c) `bridge.sh` failed build = FAIL; (d) `local-smoke.sh` per-gate log under `mktemp -d`, exit 77 = SKIPPED; (e) `carrier.sh --smoke` log under `$T111_SCRATCH` when set, `ss`/`pgrep` in the precheck; (f) `gen.test.ts` fails when `openapi-typescript` is absent | the six scripts/tests named | per item a hermetic proof (a forced abort prints FAIL and exits 1), shellcheck | shell (tiny each) |
| X6 | Shared emitter (F3 phase 2) | `verify/lib/result.sh`; the `ok`/`fail`/`skip` helpers in `smoke.sh`, `panel-lint.sh`, `6-transport/verify/lib.sh`, `spine/node.sh` | hermetic `verify/tests/result-sh-gate.sh`; text output byte-identical with `T111_RESULTS` unset | shell |
| X7 | `check.sh` keep-going: `CHECK_KEEP_GOING=1` runs every stage and still exits 1 | `1-bootstrap/scripts/check.sh`, `scripts/tests/check-summary-gate.sh` | the summary gate gains the keep-going cases | shell |
| X8 | CI job `cross-layer` and the widened shellcheck | `ci.yml`, `plan/cross-layer/ci-cd.md` | the job green on a PR, `results.jsonl` artifact present | tiny |
| X9 | Docs: how an agent reads a failure; pointers | `verify/README.md`, `AGENTS.md` (one line), `plan/README.md` row, `plan/cross-layer/test-system/TASKS.md` | `build.py --check`; lint resolves every path named | docs |
| N1 | Contract routes, both directions | `1-bootstrap/crates/node111/tests/dashboard_auth.rs` | `cargo test -p node111 --test dashboard_auth` | Rust |
| N2 | Agent stub-daemon fixtures validated against the contract | `3-intelligence/crates/agent111/tests/{daemon_tools,layer6_tools,task_engine}.rs`, `tests/fixtures/daemon/`, `4-interface/ui/test/agent-fixtures.test.ts` | agent111 `check.sh`; vitest | Rust |
| N3 | L6/L7 tapes read | `4-interface/ui/test/tapes.test.ts` | vitest | tiny |
| N4 | G7 inside S1 | `4-interface/verify/smoke.sh` | S1 GREEN with `G7 GREEN` in it | shell |
| N5 | Independent vector legs and `--selftest` in the catalog | `verify/checks.toml` | each green via `run.py --check` | tiny |
| N6 | LXMF fake parity transcript | `6-transport/sidecars/lxmf-sidecar/tests/`, `1-bootstrap/crates/node111/tests/fake_lxmf_sidecar.py` | sidecar `run-tests.sh` | shell/Python |
| N7 | Presence file schema on three sides | `5-messaging/sidecars/meshtastic-presence/{presence.schema.json,tests/}`, both `stub-meshmud.py`, `daemon/meshtastic/presence.rs` tests | sidecar tests; `cargo test` presence | shell/Python (+ a tiny Rust test) |
| N8 | Smoke's payload from `gen-manifest.py` | `4-interface/verify/smoke.sh` | S1 GREEN | shell |
| N9 | Real agent → L6 tool in G1 | `6-transport/verify/bridge.sh`, `6-transport/verify/root.sh` | G1 GREEN | shell |
| N10 | Watchdog unit test; `systemd-analyze` no longer `\|\| true` | `daemon/watchdog.rs`, `1-bootstrap/scripts/check.sh` | `cargo test -p node111 watchdog`; summary gate | Rust |
| N11 | Prompts name real tools | `3-intelligence/crates/agent111/tests/chat_loop.rs`, `tests/fixtures/tools.json`, `4-interface/ui/test/prompts.test.ts` | agent111 tests; vitest | Rust |

Order: X1–X4 (useful on day one with parsing alone), X5, N5, N3, N1, N2, N4, X6–X9, N6–N11. Disjoint rows run in parallel per
`EXECUTION.md` (X2–X4 share `verify/t111/`: serial); every row leaves nothing running and keeps its scratch in the session scratchpad.

## 7. Risks and open points

- Parsing is as stable as the harness wording (X3's goldens pin it; X6 removes it). Runner bugs could hide failures: its tests
  carry positive controls (a planted FAIL must surface), and every gate stays runnable by hand with unchanged output. Seam ids in
  prose can drift from the map; the lint checks the map against files, and the Layer 8 and 9 specs cite ids.
