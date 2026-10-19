export const meta = {
  name: 'layer-6-wave-1b-bounce',
  description: 'Layer 6 Wave 1 bounce: fix the fable review findings on B4 (transport spine) — one opus agent on the transport files only',
  phases: [{ title: 'Bounce', detail: 'B4 majors (rnstatus parser panic, undemanded rnstatus spawns) + minors; lead reviews + commits' }],
}

const ROOT = '/home/user/os'
const SP = '/tmp/claude-0/-home-user-os/e7852691-6eb6-50f1-9e84-b01d744ea3a9/scratchpad'

const CTX = `
You are a BUILD subagent for the 111network OS, Layer 6 (Transport & Bridge). Today is 2026-10-19. Repo root: ${ROOT}
(a git repo on branch claude/layer-6-implementation-006mte; you NEVER run git commit/add/stash/checkout/reset — the lead reviews
and commits each task). You will not be messaged mid-task; everything you need is in this brief and on disk.

THE LAW: the locked spec ${ROOT}/60-transport/README.md (v1.0, D27) and your task row in ${ROOT}/60-transport/TASKS.md. Read your row
first, then the spec sections the row names (the spec is 888 lines; read it with sed -n ranges), then the as-built code you build on
(named in your brief; Wave 0 landed: store V3 in mesh111 (M/store/foreign.rs), the contract in 40-interface/api/openapi.yaml, the
A10 bytes + N/bridge/attest.rs, the supervisor's piped stdio + N/components/, the KB/licensing rows). The line-verified seam map is
${ROOT}/plan/layers/60-transport/research/00-code-seams.md (line numbers from 2026-10-19; re-check before editing). Decisions:
${ROOT}/plan/DECISIONS.md D23, D25, D26, D27. Do not re-open decisions; implement them. If the spec is ambiguous, pick the reading that is
safest (fail closed) and record it under "deviations". Owner rule in AGENTS.md: the Raspberry Pi is the floor — add no resident
process, poll, timer or runtime dependency to the base configuration; everything third-party is an on-demand component with zero
runtime weight while off; the bridge core is inert with no adapter enabled.

FILE OWNERSHIP IS STRICT: create or modify ONLY the files/dirs your brief lists under "Owned" (plus new files inside those dirs). Other
tasks run in parallel on other files right now; if you need a change elsewhere, do NOT make it — describe it under "deviations" and
continue (exception: a tiny additive \`pub mod\` / registration line a new module needs — do it and list it). Never touch
plan/, TASKS.md boards or INDEX.md unless your brief names them.

HOUSE RULES: Rust — edition 2021, typed errors (never panic on input; sizes checked before parse), doc comments citing the spec section
("§x.y"), unit tests in-module and integration tests under tests/, rustfmt + clippy -D warnings clean; no new dependencies beyond the
crate's Cargo.toml unless your brief names one (node111 policy: ${ROOT}/10-bootstrap/TASKS.md "Dependency policy"; mesh111's and id111's sets
are final) — if you truly need one, do not add it, report it. No wall clock in any ordering or cryptographic decision (local_seq and the
monotonic tick only). Nothing downloads at runtime. No Reticulum code or imports anywhere in Rust. Foreign text is untrusted everywhere.
Never put a model name or identifier into any file. Never commit.

BOX: 4 CPUs, 15 GB RAM, a fixed disk allowance (~20 GB free; do NOT cargo clean, do NOT create another target dir). Shared WARM target
dirs, one per workspace, which cargo's own lock serializes between agents: 10-bootstrap (node111) → CARGO_TARGET_DIR=${SP}/target/node111 ·
50-messaging (mesh111) → ${SP}/target/mesh111 · 20-identity (id111) → ${SP}/target/id111 · 30-intelligence (agent111) → ${SP}/target/agent111.
Always export the right one before any cargo command, with CARGO_BUILD_JOBS=2. Iterate with narrow commands (cargo test -p <crate> --lib
<module>, cargo check) and run the full check once at the end. A build that fails only because of a file another task owns (another agent
is mid-edit; node111 compiles mesh111 and id111 from disk): wait 90 s, retry twice, then report "blocked-by: <task>" — never touch that
file. Toolchain: cargo 1.97 stable (rustfmt, clippy); NO musl cross toolchains here (musl / G2 stages SKIP — say so, never fake them);
node 22 + pnpm (UI deps installed); python 3.13 with venv + pip (PyPI reachable through the proxy); crates.io reachable; cc 13; jq, curl,
nc, strace, ip (iproute2), systemd-analyze present; unprivileged netns work (unshare -Urnm); no rnsd installed on the host. Scratch
files go under ${SP}/<task-id>/ only. Linux hygiene: nothing of yours stays running when you return (ps check), kill what you started,
delete your scratch.

RUN YOUR CHECK (the row's "Check" column, as refined in your brief) until it is green; paste its tail.

RETURN (structured): task id, status (done | partial | blocked), files changed/created (exact paths), the check command + its tail
(≤ 25 lines), deviations (each with why), notes for the reviewer (what to read first, open questions). Factual; the lead reads diffs.
`

const BRIEF = `TASK B4-bounce — close the fable review findings on B4 (commit 39483b0, the transport spine). Crate ${ROOT}/10-bootstrap/crates/node111 (N/ = src/daemon/). Other agents are building N/bridge, N/meshtastic and N/devices right now: touch ONLY the files below. Every API change is ADDITIVE (C1 and C4 are coding against the Spine as committed).
Owned: N/transport/{rnstatus.rs, config.rs, interfaces.rs, join.rs, posture.rs}, tests/transport_spine.rs, tests/fixtures/rnstatus/*, tests/fixtures/rns/* (only if a golden must change — say why), ${ROOT}/60-transport/verify/spine.sh (only if a step changes).
Read first: git show 39483b0 --stat; the B4 row in ${ROOT}/60-transport/TASKS.md; N/transport/rnstatus.rs (size_at ~:236, wanted() ~:588, tick ~:653, snapshot ~:545, record_down), N/transport/config.rs (load_or_mint ~:233, write_if_changed ~:650), N/transport/interfaces.rs (migration ~:1537), N/transport/join.rs (apply_join ~:226), N/transport/posture.rs (render_claimed ~:502). AGENTS.md: the Raspberry Pi is the floor — no undemanded periodic process spawn in the base configuration.
Findings to close (each with a test that fails without the fix; cite the finding id in the test name or a comment):
(F1, MAJOR, rnstatus.rs size_at) the old-form branch slices before[start..] with start = rfind(..)+1; a multi-byte char there (the other arrow, any non-ASCII byte rnsd prints) is not a char boundary and the parser PANICS on untrusted output. Fix: compute start as the byte index after the matched char (rfind + len_utf8, or char_indices); the parser must be fail-closed on any input (fuzz it with a few hundred random byte strings and arrow/non-ASCII mixes in a unit test: never panics). Add fixture/unit cases: both arrows on one line, a non-ASCII prefix.
(F2, MAJOR, rnstatus.rs wanted()/tick) wanted() is true whenever last_ok is None regardless of demand; with share_instance = false every reading answers NoInstance so last_ok never becomes Some and the tick spawns a Python rnstatus every backoff_max (5 min) for the daemon's lifetime with nobody reading. Fix: gate the tick on DEMAND only (drop the last_ok.is_none() clause; the rnsd.log heuristic covers the display state) and skip the reading outright while the file's share_instance is false (no control RPC exists then). Test: with share_instance = false and no reader, a fake runner records zero rnstatus runs over many ticks; with a reader in the last 10 min, runs happen; after 10 min without a read, they stop.
(F3, minor, config.rs load_or_mint) any read error on secrets/rns-rpc-key is treated as malformed and the key is silently rotated (rnsd restarts, the helper's client config keeps the stale key). Fix: re-mint only on Absent or on a parsed-but-invalid value; propagate other I/O errors as ApplyError::Write so nothing rotates. Test with an unreadable file (0o000; skip the test as root if the box runs as root — say so in the test).
(F4, minor, interfaces.rs migration) migration strips non-allowlisted keys of an enabled v1 RNode stanza, including network_name/passphrase (IFAC) and interface_mode, so a later region set renders it onto an unprotected network with a different mode. Fix: when an RNode stanza carries IFAC or mode keys, skip it with a note ('add it again in Settings → Layer 6') instead of migrating a weakened copy. Test.
(F5, minor, config.rs write_if_changed) renders the defaults when interfaces.toml is unreadable or invalid (unwrap_or_default), so a future caller would overwrite a user's stanzas with an empty config. Fix: fail closed on a read error (return Err), or delete the function and move its two tests onto Spine::write_config — pick the smaller change and keep tests/transport_venv.rs passing without editing it.
(F6, minor, join.rs apply_join) the IFAC passphrase is written into secrets/ before spine.update; a refused or failed update leaves an orphaned client secret. Fix: store the secret only after the update succeeds, or remove it on error. Test.
(F7, minor, rnstatus.rs snapshot/record_down) snapshot() nudges the tick on every call while rnsd is down because record_down never sets last_attempt, so each API request runs each_tick (an rnsd.log read). Fix: set last_attempt in record_down (or skip the nudge when the cache is marked down). Test.
(F8, minor, posture.rs render_claimed) RNode ports are claimed for rnsd before write_rendered; when the write fails the claims stay held. Fix: release the newly claimed keys on the error path (or claim after the write succeeds). Test with a write failure (read-only dir; skip as root if needed).
(F9, recorded, not yours) the region table is hand-transcribed; the KB-generated table is a V3/C4 item — do not touch REGIONS.
Also: Rendered derives Debug with the rpc_key in text — add a manual Debug that redacts text (one-liner) and a test that the Debug output never contains the key.
Check: export CARGO_TARGET_DIR=${SP}/target/node111 CARGO_BUILD_JOBS=2; cd ${ROOT}/10-bootstrap && rustfmt --edition 2021 --check <your files> && cargo clippy -p node111 --lib --tests -- -D warnings (if it trips only in N/bridge, N/meshtastic or N/devices — other agents' in-flight files — say so and show your files are clean with the plain clippy output) && cargo test -p node111 --lib transport && cargo test -p node111 --test transport_spine --test transport_venv --test api_v1_devices && bash ${ROOT}/60-transport/verify/spine.sh. If a build fails only in another agent's file, wait 90 s and retry twice, then report blocked-by with what you could run; never touch those files.`

const RESULT = {
  type: 'object',
  required: ['task', 'status', 'files', 'check', 'deviations', 'notes'],
  properties: {
    task: { type: 'string' },
    status: { type: 'string', enum: ['done', 'partial', 'blocked'] },
    files: { type: 'array', items: { type: 'string' } },
    check: { type: 'string' },
    deviations: { type: 'array', items: { type: 'string' } },
    notes: { type: 'string' },
  },
}

phase('Bounce')
log('Layer 6 Wave 1 bounce: B4 review majors + minors')
const r = await agent(CTX + '\n\n' + BRIEF, { label: 'L6:B4-bounce', phase: 'Bounce', schema: RESULT, model: 'opus', effort: 'high' })
log('returned: ' + (r ? r.status : 'null'))
return r
