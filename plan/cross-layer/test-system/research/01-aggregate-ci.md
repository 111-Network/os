# Scout 01: aggregate gates and CI (read-only survey, 2026-11-11)

Nothing was executed; every fact is from source, `file:line` relative to `/home/user/os`. Counts are `grep -c` of
`pass`/`expect_*` call sites or `def test_` lines, so they are call sites, not run-time case counts, unless stated.

**Summary.** There is no single entry point. The nearest is `1-bootstrap/scripts/check.sh` (185 lines, POSIX sh, `set -eu`):
it runs the Layer 1 gate (fmt, clippy, `cargo test` of node111, shellcheck, four hermetic "gates that test the gate", notices,
systemd-analyze), then nests `5-messaging/scripts/check.sh` (the mesh111 workspace), the G2 SQLCipher static link, the presence
sidecar, and the UI `pnpm check`. It stops at the first failure (`set -e`), names no failing check except through the cargo/pnpm
output of the stage that died, and ends with one summary line (`ALL GREEN` or `GREEN (... SKIPPED - not a lock run)`).
Layers 2 and 3 have their own gate (2: CI-only cargo steps, no script; 3: `scripts/check.sh`); Layer 0 is `build.py --check`;
the LXMF sidecar, the Monocypher C harness, the netns harnesses (`4-interface/verify`, `6-transport/verify`, `7-relay/verify`)
are separate and are NOT run by `check.sh`; `local-smoke.sh` runs L4 smoke only. CI (`.github/workflows/ci.yml`, 143 lines) has
four jobs, all of which skip musl/G2 and every netns harness. The only layer-to-layer seams that the aggregate really
exercises are the in-process Rust integration tests inside node111 (`cargo test`, 35 `tests/*.rs` files), which drive
node111 against `id111`, `mesh111` (path deps) and against FAKE children (sh scripts for `111-agent`, `llama-server`,
`fake_lxmf_sidecar.py`, `fake_deltachat_rpc.py`). The real `agent111` binary and the real Python sidecars are never
started by any aggregate gate. No gate emits machine-readable output (no JSON/JUnit/TAP anywhere in this area).

## Matrix: what each aggregate gate joins (exercised = runs both sides; compiled = only built together)

| Gate | Layers touched | Seam really exercised | Only compiled / stubbed |
|---|---|---|---|
| check.sh `cargo test` (node111) | 1,2,5,6,7 (+4 routes, 3 via fakes) | node111 <-> `id111` crate API (identity, sealer, announce); node111 <-> `mesh111` crate API (fabric, store, bridge); HTTP `/api/v1/*` served by node111 and hit over TCP by the tests | `111-agent`, `llama-server`, rnsd, LXMF helper, deltachat, meshtastic = sh/py fakes; UI `dist` is `allow_missing` (`daemon/dashboard/routes.rs:56`) and the UI is built AFTER this stage |
| check.sh G2 | 5 + 1 | static-link assertions on mesh111 test binaries, x86_64 binaries are RUN | aarch64 linked only |
| check.sh ui | 4 | typecheck, vitest (mock adapter), build, budget | no live daemon; contract drift is NOT here (CI only) |
| 3-intelligence check.sh | 3 | agent111 alone | wiring to node111 is tested from the node111 side with a fake agent |
| netns harnesses (not in aggregate) | 1,3,4,5,6,7 | real `111d` + real `111-agent` + stub model | out of scope here (other scouts) |

## Items

### CI-1 `1-bootstrap/scripts/check.sh` (the aggregate gate)
- **Order (stage header printed as `== name ==`)**: fmt `:36` -> clippy host `:38` -> musl check x86_64/aarch64 `:40-48` (only if `musl_toolchains_present`)
  -> `test (host)` `:49` -> shellcheck `:51` -> gate assertions `:60-67` (musl-env-test, check-summary-gate, cross-build-ui-gate,
  cross-build-asserts-gate) -> notices `:68-79` (`gen-notices.py --check`, two `unittest discover` runs) -> build scripts `:80-88`
  (`build-scripts-gate.sh`, needs `git`) -> systemd-analyze `:89` -> messaging `:102-104` -> G2 `:106-159` -> presence sidecar
  `:160-167` (`run-tests.sh` + `gen_lock.py --verify`) -> ui `:168-178` (`pnpm install --frozen-lockfile` if no
  `node_modules/.modules.yaml`, then `pnpm check`).
- **Run**: `sh 1-bootstrap/scripts/check.sh` (any cwd; it `cd`s to the layer root `:16`). Switches: `REQUIRE_G2=1` (`:31`, FAIL
  before any stage when cross toolchains absent: `FAIL: REQUIRE_G2=1 but the musl cross toolchains are absent ...` `:32`),
  `CARGO_TARGET_DIR` honoured. Needs cargo, shellcheck, python3, git, pnpm, `~/.local/toolchains/{x86_64,aarch64}-linux-musl-cross`
  for G2, `minisign` CLI (node111 update tests sign fixtures: `tests/common/mod.rs:56-69`). Runtime: CI sets `timeout-minutes: 90`
  (`ci.yml:34`); `ci-cd.md:46` says warm CI > 25 min is a smell; no measured local runtime is stated in the files read.
- **Prints / failure**: headers only; no per-stage timing, no stage table, no log file. A failure is the failing tool's own output
  and a non-zero exit; the last `== ... ==` header above it is the only stage name. Skip lines: `!! musl cross toolchains absent ...
  musl checks SKIPPED` `:46`, `shellcheck not found - SKIPPED` `:57`, `python3 not found - notices check SKIPPED` `:77`,
  `git not found - build-scripts gate SKIPPED` `:86`, `systemd-analyze or the units absent - SKIPPED` `:99`,
  `python3 not found - sidecar checks SKIPPED` `:165`, `pnpm not found - UI checks SKIPPED ...` `:176`,
  `!! musl cross toolchains absent - G2 (SQLCipher static link) SKIPPED; REQUIRE_G2=1 makes this a failure` `:158`.
- **Summary line (`:179-185`)**: no musl -> `GREEN (G2 SKIPPED - not a lock run)` + `; also skipped: <list>`; musl but another
  skip -> `GREEN (<list> SKIPPED - not a lock run)`; none -> `ALL GREEN`. Exit 0 in all three; the CI step cannot tell a lock run
  from a skipped one except by reading the last line. The exact G2 success lines: `ok: N x86_64 test binaries static X86-64
  (no INTERP/NEEDED); M carry SQLCipher (cipher_plaintext_header_size) + OpenSSL <ver>, the mesh111 lib tests among them`
  `:152` (aarch64 twin `:156`); failures: `FAIL: no <triple> test binaries were linked` `:139`, `FAIL: the mesh111 lib-test binary is
  not among the <triple> test binaries` `:140`.
- **Weak points**: (1) `systemd-analyze verify ... | grep -v "..." || true` (`:95-96`) can never fail: the stage is advisory.
  (2) `shellcheck` scope is only `shim termux verify scripts` of 1-bootstrap plus two L5 scripts (`:53-55`); NOT
  `3-intelligence/scripts`, `4-interface/verify/*.sh`, `6-transport/verify/*.sh`, `7-relay/verify/*.sh`, `hooks/pre-commit`,
  the Monocypher `.sh` (bash). `find ... 2>/dev/null` hides a missing dir. (3) Stops at first failure, so one run reveals one
  broken stage. (4) The nested `5-messaging/scripts/check.sh` prints its own `check.sh: green (musl checks SKIPPED)` mid-stream
  `:41`; only the LAST line counts. (5) Root box: 8 node111 tests fail as root (`6-transport/TASKS.md:80`: "only the 8 root-only test
  failures of a root-run box"; `api_v1_devices.rs:1507` skips itself as root, the others do not), which makes `cargo test` red on any
  container running as root (this session is root). Recorded, not counted (7-relay/TASKS.md V8, D29). (6) `cargo test` runs before
  the UI build, so the embedded dashboard served by the Rust tests is the missing-dist fallback. (7) Contract drift
  (`pnpm gen:types` + `git diff`) is CI-only (`ci.yml:124-127`), not in `check.sh`.

### CI-2 `1-bootstrap/scripts/tests/check-summary-gate.sh` (tests check.sh's last line)
- Hermetic: copies the real `check.sh` + `musl-env.sh` into `mktemp -d /tmp/111-check-gate.XXXXXX` (`:23`, removed by trap), every other stage
  a stub, a stub `cargo` that emits fake `compiler-artifact` JSON for fake binaries, a fake `readelf` (`tests/fakes.sh`), PATH =
  links to a fixed host-tool list (`:57`). Run: `sh 1-bootstrap/scripts/tests/check-summary-gate.sh`; no root, no netns, needs python3.
- Proves (13 call sites, `grep -cE` of `pass|expect_red`): full run -> `ALL GREEN` + both `ok: 2 ... static` lines + x86_64 test run in cargo.log `:108-117`;
  tools absent -> `GREEN (shellcheck, build-scripts gate, systemd-analyze, ui SKIPPED - not a lock run)` `:119-125`; no toolchains
  -> `GREEN (G2 SKIPPED - not a lock run); also skipped: musl checks`, no `--target` build `:127-135`; one toolchain only -> G2
  skipped `:137-146`; `REQUIRE_G2=1` bare -> `FAIL: REQUIRE_G2=1` before any stage `:148-154`; five red cases (aarch64 lib without
  codec, x86_64 lib without OpenSSL 3.6.3, DT_NEEDED, PT_INTERP, no mesh111 lib binary) `:167-183`; messaging check.sh with
  one toolchain -> `check.sh: green (musl checks SKIPPED)`, with both -> `check.sh: all green`, both musl checks and the pinned CC `:185-217`.
- Output: `check-gate: ok: <name>` / `check-gate: FAIL: <name>` (stderr) with a 25-line tail on failure (`show`, `:101-103`);
  summary `check-gate: ALL CASES GREEN` `:220` or `check-gate: N case(s) failed` `:223`; exit 0/1. Failing case is named. No log file.
- Weak: proves the summary logic against stubs, not that real stages run; a new stage added to `check.sh` without a stub in
  this tree makes the gate fail for an unrelated reason (the tree is hand-built `:32-55`).

### CI-3 `1-bootstrap/scripts/tests/cross-build-asserts-gate.sh` (release asserts of `cross-build.sh`)
- Hermetic tree in `/tmp/111-xbuild-asserts.XXXXXX`; stub `cargo` "builds" a fake binary per target and logs its env. Run: `sh <path>`.
- Proves (12 call sites): static x86_64+aarch64 with SQLCipher + OpenSSL 3.6.3 pass `:90-95`; the env cargo saw has every SQLite/OpenSSL
  escape hatch unset and CC/AR/linkers pinned `:97-113`; refusals for plain SQLite (only mesh111's own strings) x2, another OpenSSL version,
  system libcrypto, PT_INTERP, DT_NEEDED `:115-135`; no mesh111 in Cargo.lock -> `SQLCipher assert not applicable` `:137-140`; mesh111 without
  openssl-src -> `Cargo.lock has mesh111 but no openssl-src` `:142-147`.
- Output: `asserts-gate: ok: ...` / `asserts-gate: FAIL: ...` plus the full captured output on stderr; `asserts-gate: ALL CASES GREEN` `:150`; exit 0/1.

### CI-4 `1-bootstrap/scripts/tests/cross-build-ui-gate.sh`
- Hermetic `/tmp/111-xbuild-gate.XXXXXX`; proves (6 `pass` sites) no pnpm + no/stale `dist` refused before cargo (`dist is empty`, `dist is stale`) `:53-74`,
  fresh dist proceeds `:76-84`, `--allow-no-ui` labeled `NOT a release build` `:86-93`, pnpm rebuilds `:95-102`, no musl toolchains refused before the UI build
  (`musl cross toolchains absent`) `:104-114`. Output `ui-gate: ok:`/`FAIL:`, `ui-gate: ALL CASES GREEN` `:117`. Uses `touch -d` (GNU).

### CI-5 `1-bootstrap/scripts/musl-env-test.sh` (self-test of `musl-env.sh`)
- Run from check.sh `:62`; `set -eu`, `mktemp -d`. Proves: escape hatches (plain + per-target) unset, CC/AR/linkers pinned, same values exported
  by `5-messaging/scripts/check.sh` (grep `export $kv`) `:15-33`; `openssl_src_version` `:35-39`; `sqlcipher_linked` refuses own-strings, no-OpenSSL,
  other OpenSSL, no-SQLCipher and accepts both `:41-58`; `static_elf` refuses a dynamic host ELF and a non-ELF `:60-77`.
- Output: `FAIL: <msg>` per failure (stderr), success `ok: musl-env.sh - escape hatches unset, ...` `:87`. Hard FAIL (never skip) when `readelf`
  or a dynamic host binary is missing `:71,:75`.

### CI-6 notices and build-script gates (Layer 1 scripts guarding Layer 5 pins)
- `python3 -I scripts/gen-notices.py --check` (offline lockfile-hash compare against THIRD-PARTY-NOTICES.md; fix: run without `--check`, needs cargo+pnpm);
  `unittest discover -s scripts/notices/tests` (`test_layer5_notices.py`, 23 `def test_`) and `-s scripts/tests -p 'test_*.py'` (`test_notices_native.py`, 9) `:72-75`.
- `scripts/notices/tests/build-scripts-gate.sh` (12 `expect_*`/pass sites): meshmud and meshtastic-web build scripts' `--verify-only` paths on synthetic
  tarballs: tarball sha256 mismatch, commit pin mismatch, TODO pin is a hard stop, go.sum drift, "Go is CI-only", incomplete asset, links in asset.
  Output `build-gate: ok:`/`build-gate: FAIL: <name>: <detail>`; exit 1 on `build-gate: N failure(s)`. Hermetic, no network.

### CI-7 `5-messaging/scripts/check.sh` (mesh111 workspace gate; run alone or nested in CI-1)
- fmt `:22`, clippy `-D warnings` `:24`, `cargo test --workspace` `:26`, then musl check x86_64/aarch64 (`:35,:37`) only when BOTH cross toolchains
  (gcc+ar each) are on PATH `:29-42`. Success lines: `check.sh: all green` / `check.sh: green (musl checks SKIPPED)` (preceded by
  `!! musl cross toolchains absent ...`). PATH gains `~/.local/toolchains/*/bin`. No env switches (no `REQUIRE_*`): a skipped musl still exits 0 and the
  distinction lives only in the last line. Seam: mesh111 is a crate API consumed by node111, so the seam is tested from node111's side (CI-19), not here.
  Weak: `echo "== name =="` headers only; stops at first failure; cargo output is the whole report.

### CI-8 `3-intelligence/scripts/check.sh` (Layer 3 gate)
- 10 lines: fmt, clippy `--workspace --all-targets -D warnings`, `cargo test --workspace`, `cargo check --workspace --target x86_64-unknown-linux-musl`, then
  `check.sh: all green`. NO skip logic: with the musl target not installed it FAILS (CI installs the target and `musl-tools`, `ci.yml:84-93`). No headers between stages at all.
  Board: 93 tests across 7 suites at build (`3-intelligence/TASKS.md:66`). Agent-to-daemon seam (`daemon_tools.rs`, stub daemon) is inside this workspace; agent <-> node111 is
  tested only with a fake `111-agent` (`node111/tests/intelligence_children.rs:1-4,:30-33`).

### CI-9 `5-messaging/sidecars/meshtastic-presence/run-tests.sh` (+ `gen_lock.py --verify`)
- `python3 -I -m pytest -q -p no:cacheprovider tests` if pytest importable, else `unittest discover`; `PYTHON=<venv>/bin/python` to include `test_real_library.py`, which
  self-skips (`skipUnless(HAVE_LIB ...)`) without meshtastic-python. 97 `def test_` lines in `tests/` (3 files). No network/hardware. Proves: presence ingest hardening,
  no send path, lock/manifest structure. Output = pytest/unittest standard (`-q` dots, failing test id printed by the runner), no log path. A skipped real-library test is
  silent under `-q` unless `-rs`. Layer join: Python sidecar contract with the node111 ingest (JSON schema) is checked from the sidecar side only.

### CI-10 `6-transport/sidecars/lxmf-sidecar/run-tests.sh`
- Same runner shape (`unittest discover -s tests` / pytest -I). 128 `def test_` lines (matches the 128 on `6-transport/TASKS.md:80`); fake RNS/LXMF surface (`tests/_support.py`);
  two `test_lock.py` tests `skipUnless(os.path.exists(COMPONENT), "the assets copy lands with C3rs ...")` (`:50,:57`). Run by CI only in `knowledge-contract-hygiene` (`ci.yml:114-115`);
  NOT in `check.sh`, NOT in `local-smoke.sh`. Real RNS/LXMF is never loaded.

### CI-11 `1-bootstrap/verify/local-smoke.sh` (the "locally runnable" aggregate, T16)
- `sh verify/local-smoke.sh` from the layer root; 19 `gate "..."` sites (13 L1 + 4 L5 + V8-variant + L4 smoke). Runs: F1-F6 / V5 / V12 as single `cargo test --test X name`,
  shim harness, `cross-build-ui-gate`, V13 p_align (re-runs `cross-build.sh --allow-no-ui 2026.10.12 /tmp/dist-test` with output discarded `:40-42`), termux dry-run, FULL `cargo test --quiet`,
  `5-messaging/scripts/check.sh`, `manifest_layer5` test, `build-scripts-gate.sh`, presence sidecar, `linux-e2e.sh --namespace` (only if `unshare -rm true`), L4 `smoke.sh` (only if pnpm and `unshare -rn true`).
  Not included: `check.sh` itself, G2, UI `pnpm check`, notices, the 4-interface/6-transport/7-relay harnesses beyond L4 smoke, LXMF sidecar, Monocypher.
- Output: `smoke: PASS  <name>` / `smoke: FAIL  <name> (see /tmp/smoke-gate.log)` `:20-22`; final `smoke: ALL LOCAL GATES GREEN` `:78` or `smoke: N gate(s) failed` `:81`; exit 0/1.
  Skips print `SKIPPED ...` and are not counted.
- **Weak points**: (1) ONE shared log `/tmp/smoke-gate.log` overwritten by every gate (`:19`): after a FAIL, the path in the message is overwritten by the next gate. (2) `linux-e2e.sh` exits 77 for a missing
  dist binary (`linux-e2e.sh:39-42`) but `gate()` treats any non-zero as FAIL, so a missing `/tmp/dist-test` counts as red, not skip. (3) hardcoded `/tmp/dist-test` and release `2026.10.12`.
  (4) several gates run `cargo test` several times over the same tests (F1-F6 then the full suite): slow and double-reports a failure. (5) does not stop at first failure (good) but gives only PASS/FAIL per gate.

### CI-12 `1-bootstrap/verify/linux-e2e.sh --namespace` + `fixture_server.py` (V8 variant)
- Layers: 1 only (installer <-> signed channel); touches 2 (identity files 0600). `unshare -rm` mount namespace, real musl `111` from `/tmp/dist-test/111-x86_64-unknown-linux-musl` (must be pre-built by
  `scripts/cross-build.sh 2026.10.12 /tmp/dist-test`), fixture channel on `127.0.0.1:11180` signed with the `minisign` CLI and the test key, stubbed `systemctl/udevadm/useradd/...`. Needs unprivileged user
  namespaces + `minisign` + python3. No args prints the VM runbook (V8 blocker). Exit 77 = SKIP (`:37-41`).
- Asserts (15 `check` lines `:107-121` + three 0600 perms `:122-130`): unit, update path, udev rule, env, release binary, `current` link, five identity files, receipt, floor, offer stub, `bin/111`.
  Output `  ok: <label>` / `  MISSING: <label> (<path>)` / `  BAD PERMS: ...`; `V8-variant namespace e2e: PASS (...)` `:133` or `... FAIL`. On installer error dumps the fixture-server log. Fixed port 11180 (collides with a running dashboard).

### CI-13 `1-bootstrap/shim/tests/run.sh` (T1, POSIX shim under dash, bash, busybox sh)
- Hermetic: `python3 -m http.server` fixture on a port derived from `$$` (`:10`), token-resolved scratch copy of the shim. 9 cases x 3 shells = 27 pass sites: happy path dry-run, `--phone` x3 (escalation for binary install only, writable dest no sudo, no-privilege prints two steps),
  tampered pin refused (`sha256 mismatch`), armv7l refused, 32-bit userspace warns and uses kernel arch, no-privilege `su -c` line, truncated shim never verifies. Output `t1-test: ok: <shell>: <case>`; failures `t1-test: FAIL: ...` plus the output;
  `t1-test: ALL CASES GREEN (dash, bash, busybox sh)` `:227`. NO skip logic: a missing `dash`/`busybox` fails the case. In `local-smoke.sh` and shellcheck, NOT a `check.sh` stage and NOT in CI. Seam: shim <-> `111` installer CLI contract (`install --min-installer-version=...`) with a FAKE installer.

### CI-14 `1-bootstrap/termux/tests/dryrun.sh` (T13)
- Fake `$PREFIX`/`$HOME`, PATH-injected stubs for `termux-wake-lock`, `runsvdir`, `sv`, `curl`, python http.server for the e2e stub. 8 pass sites: boot script order and idempotence, runit `run` execs the daemon + throttle marker,
  healthcheck silent when healthy / failure log when not (the "Layer 9 seam"), `e2e-stub.sh` <= 4 KB, fetch-verify-handoff, sha mismatch refuses. Output `t13-test: ok:/FAIL:`, `t13-test: ALL CASES GREEN`. Run via `local-smoke.sh` only, not CI, not `check.sh`.

### CI-15 `2-identity/monocypher-harness/run.sh` and `mutation-check.sh`
- `bash 2-identity/monocypher-harness/run.sh [--sanitize] [--log FILE] [--no-supplement]`: sha256 check of vendored Monocypher -> `cc -std=c99 -O2 -Wall -Wextra -Werror` -> inventory assert (every `test-vectors/*.json` consumed or n/a) -> run.
  Needs a C99 compiler, bash, GNU sha256sum; no network. Layers: 2 only (Rust `id111` vectors vs C), a cross-implementation seam, not a cross-layer one. Output: per-file table `FILE VECTORS CHECKS PASS FAIL SKIP RESULT` + `TOTAL` row (`harness.c:2059-2070`),
  `  FAIL [<file>] <msg>` per failing check (`harness.c:198`), final `run.sh: OK -- every file PASS` or `run.sh: HARNESS FAILED (exit N)`. The only gate in this area with a per-file table and named failing checks. Board: 411/411 PASS (`2-identity/TASKS.md:37`).
  NOT in `check.sh`, NOT in CI, NOT in `local-smoke.sh` (only referenced in `plan/HANDOFF-2026-10-15.md:87` as "no check script ... plus run.sh"). `mutation-check.sh` (17 must-die mutants + 1 equivalent, `summary: N killed, M equivalent survivors, K problems`) is evidence, "not part of the gate".

### CI-16 `0-knowledge/_meta/build.py` (Layer 0 validator, `--check`)
- `python3 0-knowledge/_meta/build.py --check` validates the vault (frontmatter, relations graph, status gates, staleness, naming, generated `llms.txt` drift) and writes nothing; no flag = regenerate the indexes in place; `--selftest` = golden fixtures
  (`_meta/fixture/{valid,invalid}`: valid clean, generation deterministic, invalid yields >= 3 errors; prints `selftest ok - ...` / `SELFTEST FAILED:`). Needs Python >= 3.11 and PyYAML (exit 2 without it, `build.py:33-35`). Output: findings grouped per file
  `  ERROR:<line>  <message>` / `WARN`, then `\nN error(s), M warning(s)` `:821`; exit 1 iff any ERROR. Failing check is named with file and line: the best-formed report in the repo.
- **Weak**: results depend on `date.today()` (`:884`): a `proven` entry older than `STALE_ERROR_DAYS` (180, `:139`) becomes an ERROR with no code change (`:662`), so a gate can go red by time alone. `--selftest` is NOT run by the hook or CI. Joins no layers (it checks the KB that Layer 1 ships as `share/kb`; the payload-manifest seam is tested in node111).

### CI-17 `hooks/pre-commit` (22 lines)
- Enabled by `git config core.hooksPath hooks`. (1) on branch `main` with an `origin`: `timeout 15 git fetch -q origin main`, refuse if `HEAD..origin/main` > 0 (message `pre-commit: main is N commit(s) behind origin/main ...`), offline = warning only; (2) `python3 0-knowledge/_meta/build.py --check`.
  Runs no Rust, no shell gate. Exit = build.py's exit. Only branch `main` is checked for staleness.

### CI-18 `.github/workflows/ci.yml` (4 jobs; triggers: pull_request, push to main, workflow_dispatch; `concurrency` cancels older runs; `permissions: contents: read`)
- **bootstrap-gate** `:28-58` (90 min): stable Rust + rustfmt/clippy, rust-cache for `1-bootstrap` and `5-messaging`, pnpm 22, `pnpm install --frozen-lockfile`, `apt install minisign`, `sh 1-bootstrap/scripts/check.sh`.
  Versus a local lead run: musl/G2 SKIP (no toolchains; `ci.yml:4-5`), so the final line is `GREEN (G2 SKIPPED - not a lock run); also skipped: musl checks`; the job is green anyway. shellcheck/systemd-analyze presence on `ubuntu-latest` is assumed, not installed by the workflow (not verified here; if absent they SKIP silently green).
  Root-only failures do not occur (non-root runner).
- **identity** `:60-77`: `cargo fmt/clippy/test --workspace` in `2-identity` only. Not the Monocypher harness.
- **intelligence** `:79-94`: adds target `x86_64-unknown-linux-musl` + `musl-tools`, `sh 3-intelligence/scripts/check.sh`.
- **knowledge-contract-hygiene** `:96-143`: `build.py --check` (PyYAML 6.0.3), LXMF sidecar tests, `pnpm gen:types` + `git diff --exit-code -- 4-interface/ui/src/lib/api` (OpenAPI -> TS drift; the only automated check of that file-format seam),
  `git grep` for two-digit layer paths (`:132`), attribution-line scan of PR commits (`:141`, PR events only).
- NOT in CI (`ci-cd.md:17-19`): musl/G2, every netns harness (`*/verify/*.sh`, `unshare -Urnm` restricted on Ubuntu 24.04 runners), G7 zero-weight, hardware gates, shim/termux/linux-e2e/local-smoke, Monocypher harness, `build.py --selftest`.
  Board V8 of Layer 7 (`7-relay/TASKS.md:113`, status open) plans to add `stub-ntfy.py` compile and shellcheck of `7-relay/verify/*.sh` to `knowledge-contract-hygiene`; `ci.yml` does not do that yet.
- Output on failure: the GitHub step log; no artifacts uploaded, no JUnit, no summary annotations except `::error::` for two hygiene rules.

### CI-19 node111 `cargo test` (the one place cross-layer seams are really driven; run inside CI-1 stage `== test (host) ==`)
- `1-bootstrap/Cargo.toml` workspace = `crates/node111` only; `id111` (`Cargo.toml:26`) and `mesh111` (`:68`) are path deps, so their own tests are NOT run here (they run in CI-18 `identity` and CI-7). 35 `tests/*.rs` files (`ls | wc`), grouped:
  HTTP/route (`api_v1_{bridge,devices,meshtastic,messages,panels,relay,status_services,transport}.rs`, `dashboard_{auth,http}.rs`, `settings_lan.rs`), bridge/fabric (`bridge_{core,deltachat,lxmf}.rs`, `carrier_ntfy.rs`, `categorize_table.rs`),
  supervisor/children (`intelligence_children.rs`, `supervise_{pdeathsig,piped}.rs`, `components.rs`, `daemon_presence_venv.rs`, `transport_{spine,venv}.rs`, `devices_core.rs`), install/update (`install_flow.rs`, `helper_guard.rs`, `updater_stage.rs`, `phone_flow.rs`,
  `manifest_{layer5,parse,trust}.rs`, `minisign_verify.rs`). Seam types: crate API (`id111::`, `mesh111::` imported in `api_v1_messages.rs`, `bridge_*.rs`, `carrier_ntfy.rs`, `api_v1_status_services.rs`), HTTP on a loopback ephemeral socket, process boundary against sh/py fakes
  (`fake_lxmf_sidecar.py`, `fake_deltachat_rpc.py`, `fake_rnodeconf`, `fake_python`), file formats (manifests, `state/*.json`).
- Needs: `python3` (Meshtastic flow tests, otherwise they FAIL unless `NODE111_SKIP_PYTHON_TESTS=1` asks to skip: `api_v1_meshtastic.rs:463-477`), `minisign`, non-root (see CI-1 weak point 5). Output: libtest default: `test <name> ... FAILED` + panic message + `failures:` list; no JSON, no log path.
- Weak: one 42-entry directory but no manifest says which test file proves which layer seam; the fakes encode the contract by hand, so a real-binary drift (`111-agent`, rnsd, `lxmf` helper) passes this gate and is only caught by the netns harnesses.

### CI-20 UI `pnpm --dir 4-interface/ui check` (`package.json:15`: typecheck && test && build && budget)
- `tsc --noEmit`, `vitest run` (board: 588 vitest tests, `4-interface/TASKS.md:43`), `vite build`, `node scripts/check-budget.mjs` (size budget, D21). `gen:types` (`openapi.yaml` -> `src/lib/api/types.gen.ts` + `scripts/gen-sse.mjs`) is separate. Joins L4 UI <-> HTTP contract only through the mock adapter and the generated types.
  Output vitest default. Skip only when pnpm absent (`:176`).

## Cross-cutting findings for the design

1. **Skip is exit 0 everywhere.** `check.sh` has a skip list in the last line only; `5-messaging/check.sh` the same; `linux-e2e.sh` uses exit 77 (SKIP) but its caller maps it to FAIL; the sidecar tests skip silently. No script emits a stable `SKIPPED:<id>:<reason>` record an agent can count.
2. **No gate writes machine-readable results**; the nearest are the Monocypher per-file table and `build.py`'s `ERROR/WARN` lines.
3. **Naming:** hermetic gates (CI-2..5, CI-6 build gate, CI-13, CI-14) name the failing case and echo the output; cargo/pnpm/pytest gates name failing tests but only inside a wall of output. `check.sh` itself names only the stage header.
4. **Scratch dirs:** `/tmp/111-check-gate.*`, `/tmp/111-xbuild-*`, `/tmp/111-shim-test.*`, `/tmp/111-termux-test.*`, `/tmp/111-e2e.*`, `/tmp/monocypher-*` are removed by `trap ... EXIT`; shared fixed paths `/tmp/dist-test`, `/tmp/smoke-gate.log` and port 11180 are not.
5. **Ordering hazards:** check.sh `cargo test` before UI build; `local-smoke` V13 depends on a prior release build in `/tmp/dist-test`; two cargo builds at most on a 12 GB box (`AGENTS.md`), so any aggregate runner must serialise cargo stages.
6. **Seams with NO aggregate coverage:** real `agent111` <-> node111 (`/relay`, token), real LXMF/rnsd <-> node111 (fakes only), 4-interface <-> daemon over a real socket and the `6-transport`/`7-relay` netns harnesses (not in `check.sh`, `local-smoke.sh` or CI), OpenAPI <-> Rust routes (the generated TS is checked, the Rust handlers are not diffed against `openapi.yaml` by any gate read here), UI <-> daemon (mock adapter only in `pnpm check`).
7. **Board vs code:** board counts (e.g. 118 smoke, 157-159 three-node, 144 panel-lint, 1147 mesh111 tests: `5-messaging/TASKS.md:50,83`, `6-transport/TASKS.md:80`) are the boards' claims; they were not recounted here (those harnesses are other scouts' area).
