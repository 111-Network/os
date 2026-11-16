export const meta = {
  name: 'layer-7-wave-3',
  description: 'Layer 7 build Wave 3 per 7-relay/TASKS.md: the netns gates G8-G12 (V2-V6) and the CI mapping (V8), authored in parallel with one harness run at a time (a shared flock); each gate gets an independent read-only review and one fix pass; the lead runs every gate with REQUIRE_* (V9) and commits each row',
  phases: [
    { title: 'Stage A', detail: 'V2 G8 presence, V5 G11 TLS-RPK, V6 G12 the hub origin, V8 the CI mapping' },
    { title: 'Stage B', detail: 'V3 G9 cross-hub forward, V4 G10 the carrier' },
  ],
}
// args: { only: ['V2', ...], sp: '<session scratchpad>', bin: '<a debug 111 built from a head with all of Layer 7>', extra: { <id>: '<lead note>' }, root }
const a = args || {}
const ROOT = typeof a.root === 'string' ? a.root : '/home/user/os'
const SP = typeof a.sp === 'string' ? a.sp : '/tmp/scratch'
const BIN = typeof a.bin === 'string' ? a.bin : `${SP}/bin-head/111`
const ONLY = Array.isArray(a.only) && a.only.length ? a.only : null
const LOCK = `${SP}/l7-harness.lock`

const CTX = `You build ONE row of Wave 3 (the gates) of the Layer 7 (Relay & Presence) build board in ${ROOT}, the 111network mesh OS. Read first:
${ROOT}/AGENTS.md (the Raspberry Pi floor, Linux hygiene, the box rules; never add attribution lines), ${ROOT}/plan/EXECUTION.md, then the board
${ROOT}/7-relay/TASKS.md: its header and box rules, YOUR row whole (Task, Criteria, Check, Files), and the status cells of the rows your gate
exercises (they are the as-built facts: each names its files, functions, figures and deviations). Spec: ${ROOT}/7-relay/README.md (locked v1.0,
D32): §15 (your gate) and every section it names, read with sed -n ranges. D33 (plan/DECISIONS.md, open) amends it: the DNS-SD service type is
_n111._tcp (the board's \`_111._tcp\` reads \`_n111._tcp\`). Contract: ${ROOT}/4-interface/api/openapi.yaml.

WHAT IS BUILT (all committed): every Layer 7 Wave 0-2 row: the twelve mesh111 amendments (M1-M6), the presence book (D2), the gateway row (D3),
netwatch (D4), the TLS-RPK link (D5), pairing (D6, D6b-d), the hub origin (D7), the registry and mDNS (D8), the role bits (D9), the relay routes
and consents (D10), the carrier on the forwarder (D11), the hub-origin client and \`111 update --from\` (L4), the KB (K3), and G7 extended
(7-relay/verify/idle.sh, row V1: GREEN). The ntfy stub server is 7-relay/verify/stub-ntfy.py (row P1). The daemon binary to run: ${BIN} (a debug
build of a head with all of the above; it reads the debug-only test switches such as NODE111_BRIDGE_STUBS). Do not rebuild the daemon for the
harness; pass BIN111=${BIN} (and give your harness the same BIN111 / CARGO_TARGET_DIR lookup idle.sh has).

HOW A GATE IS WRITTEN HERE: follow ${ROOT}/7-relay/verify/idle.sh (V1, the built model: user + net + pid namespaces with unshare, a scratch root
under TMPDIR, an inner function run inside the namespaces, traps that kill what it started, a final hygiene scan) and the helpers it sources:
7-relay/verify/lib.sh (YOUR gate's netns, veth, address and port names: use them, do not invent others; add a name to your own block there only if
your row needs one and say so) and 6-transport/verify/{lib,root}.sh (ok/note/fail/skip_or_fail/finish; the REQUIRE_<GATE>=1 convention: a missing
prerequisite prints '<GATE> SKIPPED: <reason>' and exits 0 unless REQUIRE_<GATE>=1 makes it a FAIL; the last line is '<GATE> GREEN' or '<GATE>: N
FAIL'), and 6-transport/verify/egress-log.py for outbound counts. Several daemons in one harness each get their own netns and NODE111_ROOT.
Every criterion of your row is one or more 'ok:' / 'FAIL:' lines whose text names what was observed; an item the box cannot run (a missing real
server, binary or tag) is a 'note:' plus SKIPPED for that item, never a silent pass. A check that cannot fail (asserting what the harness itself
set up) is worthless: for each item, make sure a broken daemon would turn it FAIL, and say how you know (a negative run where cheap).

ONE HARNESS AT A TIME (box rule): other Wave 3 authors run their harnesses on this box now. Run EVERY harness invocation (yours, full or partial)
under the shared lock: \`flock -w 540 ${LOCK} bash 7-relay/verify/<yours>.sh\` (with the env you need); if the wait times out, retry. Keep a
dev run short (shorter settles through your harness's own env knobs) and the lock run at the row's defaults. Never run 7-relay/verify/idle.sh.

FILE OWNERSHIP IS STRICT: create or modify only the files your row's Files column lists (plus a new fixture inside a dir it names, your own block
of 7-relay/verify/lib.sh, and your gate's entry in ${ROOT}/verify/checks.toml: when your gate runs for real, promote its l7.g<N> entry from the T0
stub (strength N) to strength R with its real needs and rerun_cost, as row V1 did for l7.g7ext; list each). Other agents edit other files of this
tree right now (Layer 8 and 9 rows, other gates). If you need a change elsewhere (a daemon bug your gate finds is the main case), do not make it:
report it under DEFECTS FOUND with the failing line, the file:line you traced it to and the fix you would make; your gate keeps the FAIL. Never
touch plan/, a TASKS.md board, INDEX.md or a spec. Shell: bash as the sibling scripts, shellcheck clean; Python: stdlib only, run with -I.

BOX: 4 CPUs; at most two cargo builds on the box and other agents hold them: run cargo only for your row's own Check line, ONE command at a time,
CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=2 TMPDIR unset, CARGO_TARGET_DIR=${SP}/target/node111 (1-bootstrap) or ${SP}/target/mesh111 (5-messaging).
Disk is limited (about 12 GB free): no new target dir unless your row says so. Scratch only under ${SP}/l7-<row>/, deleted at the end (keep the last
run's log there until you return). Hygiene: nothing of yours running or listening when you return (ps, ss, ip netns), no netns or veth left.

RETURN a report: per criterion of your row MET / PARTIAL / NOT MET / SKIPPED (why) with the harness line that proves it; the last full run's ok/
FAIL/note lines and its final line (with REQUIRE_<GATE>=1); DEFECTS FOUND (daemon bugs your gate exposes, each traced); the records your row asks
for (figures, or "not measured" and why); deviations; files changed; open questions for the lead.`

const T = {
  V2: { model: 'sonnet', brief: `TASK V2 (row "| V2 |"): G8 presence and the registry, 7-relay/verify/presence.sh (T0 stub). The facts to read: D2
(presence: N/relay/presence.rs, the \`relay::tap\` fan-out), D3 (the gateway row: N/relay/gateway.rs), D4/D4b (netwatch re-arm), D8 (N/registry/,
the mDNS record; D33's _n111._tcp; TXT v, r, k), D9 (role bits), the hops route (POST /api/v1/nearby/hops, a fake \`rnpath\` in the transport
venv: see how 6-transport/verify/*.sh fake Reticulum tools), and NODE111_BRIDGE_STUBS (6-transport/verify/bridge.sh:18-19) for the licensed board.
The nodes: hub H, members M1 and M2 on one bridge (lib.sh L7_G8_*), M1 and M2 also direct.` },
  V3: { model: 'sonnet', brief: `TASK V3 (row "| V3 |"): G9 cross-hub forward, 7-relay/verify/forward.sh (T0 stub) and its committed tape(s) under
7-relay/verify/tapes/ (the 6-transport/verify/tapes/ precedent). The facts: M1-M6 (the forwarder, the 11 B hint TLV, forward_take, the paired-hub
record), D5 (TLS-RPK on TEST-NET addresses), D6 (pairing: the scan, offer and unpair routes; the pair_plain consent), D9 (role bits). Item 6's old
hub B: the tag checkpoint-layer-6 and the board's 4689dbf are NOT in this clone or on the remote; read the old revision from G9_OLD_REV (default
checkpoint-layer-6) and SKIP item 6 when it does not resolve (FAIL under REQUIRE_G9=1); the lead will run it with G9_OLD_REV=561cb5e (the last
commit before the Layer 7 scaffold, Layer 6 complete). Building it: \`git archive\` of that revision into ${SP}/l7-V3/old, then one
\`cargo build -p node111 --bin 111\` with its own CARGO_TARGET_DIR=${SP}/target-g9old (allowed for this row only; delete it when you return) under
the cargo rules; if the disk (df) would fall under 4 GB free, do not build: item 6 SKIPs with that reason.` },
  V4: { model: 'sonnet', brief: `TASK V4 (row "| V4 |"): G10 the carrier, 7-relay/verify/carrier.sh (T0 stub), with 7-relay/verify/stub-ntfy.py
(row P1's; extend only what the gate needs, keeping P1's tests passing: tests/carrier_ntfy.rs and tests/carrier_flow.rs use it) and
7-relay/verify/logs/. The facts: D11's carrier runtime (N/carrier/{mod,ntfy}.rs: 5A drain-post-drain, 5B on the thread 111d-ntfy-5b, the metered
window, the consent owed now, the licensed refusal, the re-post rule) and tests/carrier_flow.rs (what is already proven in-process: the gate proves
it across netns with real daemons). NTFY_BIN (a pinned real ntfy release) is absent on this box: its items SKIP unless REQUIRE_NTFY=1; the smoke
log written from a stub run is logs/ntfy-smoke-<calver>-stub.md (P1's naming), never the real server's name.` },
  V5: { model: 'sonnet', brief: `TASK V5 (row "| V5 |"): G11 TLS-RPK, 7-relay/verify/tls.sh (T0 stub). The facts: D5 (N/relay/tls.rs: rustls
raw public keys, the pinned SPKI, wrap = None for tcp-lan), D6 (the plain-pairing warning and its consent), and tests/relay_tls.rs (in-process
proofs; the gate proves the wire across netns: capture with tcpdump if present, else the egress logger, that no 111 byte or announce travels in
clear on the TEST-NET veth). \`openssl version\` decides REQUIRE_OPENSSL_RPK's items (OpenSSL >= 3.2 with RPK support), else SKIPPED.` },
  V6: { model: 'sonnet', brief: `TASK V6 (row "| V6 |"): G12 the hub origin and the Layer 1 trust fixes, 7-relay/verify/origin.sh (T0 stub).
The facts: D7 (N/origin/: /origin/v1/*, served only with hub role, LAN sharing and origin.serve with its consent), L2 (the staging lock, the
origin list, the blob rule), L3 (T1/T2 manifest trust), L4 (\`111 update --from\`, the hub-origin client on attribution, check_internet), and the
signing fixtures the node111 tests use for a signed manifest (tests/updater_stage.rs, tests/manifest_trust.rs, tests/updater_origin.rs: reuse
their fixture keys and builders through a tiny test-only path rather than inventing a signer; the release key is compiled in, so a gate run
signs with the test key the debug build accepts, as those tests do; if the debug binary cannot be made to accept a test-signed manifest, the
items that need it are node111-test items (the row says T3 and every flip are) and the harness asserts the rest).` },
  V8: { model: 'sonnet', brief: `TASK V8 (row "| V8 |"): the CI mapping, .github/workflows/ci.yml and plan/cross-layer/ci-cd.md (this row may edit
that plan file: it is in its Files column). The mesh111 and node111 tests behind G8-G12 run in bootstrap-gate (1-bootstrap/scripts/check.sh
already includes 5-messaging/scripts/check.sh: check it, add nothing if so); the agent's pins in intelligence; contract drift and the KB in
knowledge-contract-hygiene, which also compiles 7-relay/verify/stub-ntfy.py (python3 -m py_compile) and shellchecks 7-relay/verify/*.sh (and
8-storage/verify/*.sh and 9-evolution/verify/*.sh where they exist, as a glob that tolerates their absence); the netns harnesses and G7 stay the
lead's (runners restrict unshare). The workflow must parse (python3 -c 'import yaml' if available, else a careful read; actionlint if present).
No harness runs for this row, so no lock is needed. The lead pushes and reads the CI run.` },
}

const RESULT_NOTE = (id) => (a.extra && a.extra[id]) ? `\n\nLEAD NOTE (read first): ${a.extra[id]}` : ''
const REVIEWED = new Set(['V2', 'V3', 'V4', 'V5', 'V6'])
const FINDINGS = {
  type: 'object', required: ['findings', 'sound'],
  properties: {
    findings: { type: 'array', items: { type: 'object', required: ['severity', 'where', 'defect', 'scenario', 'fix'], properties: {
      severity: { type: 'string' }, verdict: { type: 'string' }, where: { type: 'string' }, defect: { type: 'string' }, scenario: { type: 'string' }, fix: { type: 'string' } } } },
    sound: { type: 'array', items: { type: 'string' } },
  },
}

const review = (id, report, ph) => agent(`You are a read-only adversarial reviewer in ${ROOT} (Layer 7 build, Wave 3: the gates). Do NOT run any
harness, cargo or build, do NOT edit any file, do NOT start a process. Review row "| ${id} |" of ${ROOT}/7-relay/TASKS.md (read it whole) against
${ROOT}/7-relay/README.md §15 and the sections it names, reading the uncommitted diff of the files the author reports (cd ${ROOT} && git diff
--histogram -- <paths>; a new file: read it whole) and the daemon code the gate exercises. The author's report (its declared deviations and SKIPs
are not findings unless they hide a criterion the box can run; report what it did not declare): <<<${String(report).slice(0, 9000)}>>>
Hunt for: a check that cannot fail (it asserts what the harness set up, greps a line the harness printed, counts in the wrong namespace, races
a fixed sleep); a criterion of the row unmet, met only in a note, or silently skipped; a SKIP or a 'note:' where the box can run the item; a FAIL
path that exits 0; cleanup that leaves a process, netns, veth or listener behind (on success, on FAIL and on an interrupt); a test daemon reaching
off-box; an assertion weaker than the spec sentence it claims; shellcheck-only cleanliness hiding a quoting bug. CONFIRMED when traced end to end,
PLAUSIBLE otherwise. Report only what survives your scrutiny; list what you checked and found sound.`,
  { label: `L7:${id}-review`, phase: ph, schema: FINDINGS, model: 'fable' })

const refix = (id, report, rv, ph) => agent(`${CTX}\n\n${T[id].brief}${RESULT_NOTE(id)}\n\nYou built row ${id}; your report:
<<<${String(report).slice(0, 9000)}>>>
An independent reviewer reports: ${JSON.stringify(rv.findings).slice(0, 20000)}
Verify each against the script and the daemon; fix the real ones (each fixed check shown able to fail: a negative run where cheap); rebut the wrong
ones with evidence. Re-run your gate once at the row's defaults with REQUIRE_<GATE>=1 under the lock. RETURN per finding FIXED / REBUTTED with
evidence, the last run's lines and final line, DEFECTS FOUND, files changed.`,
  { label: `L7:${id}-refix`, phase: ph, model: T[id].model })

const row = (id, ph) => agent(`${CTX}\n\n${T[id].brief}${RESULT_NOTE(id)}`, { label: `L7:${id}`, phase: ph, model: T[id].model })
  .then(async (report) => {
    if (!report) return { id, error: 'the author died' }
    if (!REVIEWED.has(id)) return { id, report }
    const rv = await review(id, report, ph)
    if (!rv || !rv.findings.length) return { id, report, review: rv }
    const fixed = await refix(id, report, rv, ph)
    return { id, report, review: rv, refix: fixed }
  })

const want = (id) => !ONLY || ONLY.includes(id)
phase('Stage A')
const A = await parallel(['V2', 'V5', 'V6', 'V8'].filter(want).map(id => () => row(id, 'Stage A')))
log('stage A: ' + A.filter(Boolean).map(r => `${r.id}${r.review ? ' (' + r.review.findings.length + ' findings)' : ''}${r.error ? ' ' + r.error : ''}`).join(' · '))
phase('Stage B')
const B = await parallel(['V3', 'V4'].filter(want).map(id => () => row(id, 'Stage B')))
log('stage B: ' + B.filter(Boolean).map(r => `${r.id}${r.review ? ' (' + r.review.findings.length + ' findings)' : ''}${r.error ? ' ' + r.error : ''}`).join(' · '))
return { A: A.filter(Boolean), B: B.filter(Boolean) }
