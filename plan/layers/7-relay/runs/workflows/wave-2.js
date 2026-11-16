export const meta = {
  name: 'layer-7-wave-2',
  description: 'Layer 7 build Wave 2 per 7-relay/TASKS.md: D11 the carrier on the forwarder, L4 the hub-origin client, V1 the no-timer gate, D6d the pairing follow-ups, K3 the KB (after D11); each Rust or shell row gets an independent read-only review and one fix pass; the lead re-runs the checks and commits',
  phases: [
    { title: 'Stage A', detail: 'D11 (opus) beside V1 (sonnet): one cargo row at a time' },
    { title: 'Stage B', detail: 'L4 (opus) beside D6d (opus, small) and K3 (sonnet, after D11)' },
  ],
}
// args: { only: ['D11', ...], sp: '<session scratchpad>', extra: { <id>: '<lead note>' }, root }
const a = args || {}
const ROOT = typeof a.root === 'string' ? a.root : '/home/user/os'
const SP = typeof a.sp === 'string' ? a.sp : '/tmp/scratch'
const ONLY = Array.isArray(a.only) && a.only.length ? a.only : null

const CTX = `You build ONE row of the Layer 7 (Relay & Presence) build board in ${ROOT}, the 111network mesh OS. Read first: ${ROOT}/AGENTS.md (the
Raspberry Pi floor, Linux hygiene, the box rules; never add attribution lines), ${ROOT}/plan/EXECUTION.md, then the board ${ROOT}/7-relay/TASKS.md:
its header and box rules, YOUR row whole (Task, Criteria, Check, Files, the "never adds" clause), and the status cells of the Wave 1 rows you build
on (they are the as-built facts: D1-D10, A1, U1-U3; each names its files, functions and deviations). Spec: ${ROOT}/7-relay/README.md (locked v1.0,
D32; amended under D33 in plan/DECISIONS.md: the DNS-SD type is _n111._tcp); read the sections your row names with sed -n ranges. Contract:
${ROOT}/4-interface/api/openapi.yaml (binding for every route and shape; conventions 4-interface/api/README.md). Re-check every file:line the row
cites before editing: the code moved since the board was written.

WAVE 1 FACTS (all committed, 2026-11-15): D10 (d2849c4) owns N/dashboard/api_v1/relay7.rs and N/relay/{mod,consent}.rs. Its names for this wave:
\`relay::changed()\` (callable from any thread; publishes the full relay frame), \`relay::CarrierHook { reconfigure(&RelaySettings), check() -> bool,
flush() -> bool, metered_cap_reached() -> bool, licensed_board() -> bool }\` registered in ApiState ext as \`Arc<dyn CarrierHook>\` (\`api.insert_ext\`
replaces \`relay::NoopCarrier\`; \`relay::carrier_hook(api)\` reads it), \`relay7::cell(api)\` (\`settings()\`, \`replace(s)\`: replace after your own
\`rs::update\` when you persist drain cursors or the metered window), \`relay7::carrier_consented(&s)\` and \`relay::consent::holds(&s, kind)\` (the
consent owed NOW: every metered link's cap is in the carrier text, so a cap set or raised owes a new consent), \`relay7::carrier_refusal\`.
\`reconfigure\` is called after EVERY settings write, also a no-op one: it must be idempotent. D3's gateway row (N/relay/gateway.rs) and D9's role
bits already require \`consent::holds\`. D2 (presence), D4 (N/relay/netwatch.rs: the debounced address/route subscription; \`Consumer\` kinds), D5
(the pinned-key TLS link), D6 (N/relay/pair.rs), D7 (N/origin/: \`Origin::shared(api)\`, \`admit\`, \`retain_targets\`, \`bytes\`), D8 (N/registry/,
mdns), D9 (role bits; \`ROLE_ORIGIN\`) are built. M4 gave mesh111 \`set_carrier_max_frame\`, \`forward_take\`, \`carrier_inbound\` (read
5-messaging/crates/mesh111/src/fabric.rs as it is now).

FILE OWNERSHIP IS STRICT: create or modify only the files your row's Files column lists (plus a new file inside a dir it names, the one claim of a
new test file in ${ROOT}/verify/checks.toml, and a tiny registration line a new module needs: list each). Other rows run beside you on other files.
If you need a change elsewhere, do not make it: describe it under deviations. Never touch plan/, TASKS.md boards, INDEX.md or a spec.

HOUSE RULES: Rust 2021, typed errors, never panic on input, sizes checked before parse, doc comments citing "§x.y", unit tests in-module and
integration tests under tests/, rustfmt and clippy --all-targets -D warnings clean (4 known manifest unused_dependencies warnings on node111:
sysinfo, os_info, nusb, rtnetlink: ignore, do not remove); no new dependency; no wall clock in any ordering or cryptographic decision; foreign
bytes untrusted; secrets never echoed, logged or framed; every route problem+json, the agent's bearer 401 on every Layer 7 write; no model name in
any file; never commit. A new test file is claimed in verify/checks.toml (python3 -I -B verify/run.py --lint stays GREEN).

BOX: 4 CPUs; at most two cargo builds on the box and another runner may hold one: run ONE cargo command at a time. CARGO_TARGET_DIR=${SP}/target/node111
for 1-bootstrap (warm) and ${SP}/target/mesh111 for 5-messaging, with CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=2 and TMPDIR unset, from that workspace.
Iterate narrowly (cargo test -p node111 --lib <module>; cargo check), run your row's full Check once at the end, plus --lib (full) and --test
dashboard_auth when you touch a route. A build that fails inside another row's file is its half-made edit: wait 90 s, retry twice, then report
blocked. Scratch under ${SP}/<row>/ only, deleted at the end. Hygiene: nothing of yours running or listening when you return (ps, ss).

Check every new test fails without the code it tests (revert temporarily). RETURN a report: per criterion of your row MET / PARTIAL / NOT MET with
file:line; deviations; the commands you ran and their result lines; measured figures (or "not measured" and why); files changed; open questions.`

const T = {
  D11: { model: 'opus', brief: `TASK D11 (row "| D11 |"): the carrier on the forwarder. Replace D10's NoopCarrier with the real hook (a CarrierHook built only
while gateway.enabled, carrier.kind = "ntfy" and consent::holds(CarrierEnable); \`reconfigure\` builds, rebuilds or tears down idempotently and is
cheap when nothing changed). Before every flush: \`relay7::carrier_refusal\`-equivalent checks (the consent owed now, the metered budget, the licensed
board). Call \`relay::changed()\` after record_check, record_flush and set_pending (D10's note). The check and flush routes in relay7.rs then answer
from your hook ({started: true} when a flush starts). stub-ntfy.py: find the existing stub (grep -r stub-ntfy ${ROOT}; P1 or a harness may hold
one) or write tests/fixtures/l7-D11/stub-ntfy.py. Records on the row (in your report): RSS during a flush, the 5B thread, held connections.` },
  V1: { model: 'sonnet', brief: `TASK V1 (row "| V1 |"): the no-timer gate script 7-relay/verify/idle.sh (T0 stub) and baseline-wakeups.txt (new, empty
line is allowed: the row says how it behaves while empty). Read the sibling harnesses for their conventions (6-transport/verify/*.sh,
7-relay/verify/lib.sh if any, 6-transport/verify/egress-log.py) and reuse their netns helpers. Run it once on this box (unshare -Urnm netns works;
a release or debug 111 binary: build node111 with the warm target dir if needed, one cargo command) and paste its summary; shellcheck clean.` },
  L4: { model: 'opus', brief: `TASK L4 (row "| L4 |"): the hub-origin client and \`111 update --from\`. The home hub's origin counts only while its announce
carries ROLE_ORIGIN, checked once when the fabric link to it attributes (\`Fabric::attributed_on\` / the FabricEvent::Fabric edge; no timer) and on
--from. After a stage, a serving hub admits the verified files (D7's \`Origin::admit\`), then calls \`relay::changed()\` (origin.bytes moves). The
hourly internet loop honours \`updates.check_internet\` (D10 stores it in state/relay.json; read it through relay7::cell or the settings file).
GET /api/v1/update/origins per the contract. Records: the transient RSS of a staged update.` },
  D6d: { model: 'opus', brief: `TASK D6d (a small follow-up to Layer 7 rows D6 and D10; no board row of its own yet: the lead adds it): in
N/relay/pair.rs only (and its tests in tests/relay_pair.rs): (1) call \`crate::daemon::relay::changed()\` when a pairing attempt ends after its route
has answered, and after \`set_pair_link\` (a half emits no fabric event), so the relay frame shows the pair; (2) make relay::consent the one place
for the plain-pairing warning: pair.rs uses \`relay::consent::hash\` and the constant D10 re-exports (\`consent::PAIR_PLAIN_WARNING\`): move the
constant's definition into N/relay/consent.rs only if that file's owner row (D10) is done (it is) and keep the re-export so nothing else changes;
the unit test pinning hash(PAIR_PLAIN_WARNING) == pair_plain_hash_hex() keeps passing or is replaced by the single definition. Files:
N/relay/pair.rs, N/relay/consent.rs, tests/relay_pair.rs. Check: --test relay_pair, --test api_v1_relay7, --lib relay, clippy, fmt.` },
  K3: { model: 'sonnet', brief: `TASK K3 (row "| K3 |"): the KB entries, after D11 (read D11's code as it now is in N/carrier/). Follow
${ROOT}/0-knowledge/AGENTS.md and 0-knowledge/README.md (the entry format, status gates). Regenerate the indexes with build.py; its --check has 0
errors (warnings that existed before are fine: run --check before your edit and after, and report both counts).` },
}

const RESULT_NOTE = (id) => (a.extra && a.extra[id]) ? `\n\nLEAD NOTE (read first): ${a.extra[id]}` : ''
const REVIEWED = new Set(['D11', 'L4', 'V1', 'D6d'])
const FINDINGS = {
  type: 'object', required: ['findings', 'sound'],
  properties: {
    findings: { type: 'array', items: { type: 'object', required: ['severity', 'where', 'defect', 'scenario', 'fix'], properties: {
      severity: { type: 'string' }, verdict: { type: 'string' }, where: { type: 'string' }, defect: { type: 'string' }, scenario: { type: 'string' }, fix: { type: 'string' } } } },
    sound: { type: 'array', items: { type: 'string' } },
  },
}

const review = (id, report, ph) => agent(`You are a read-only adversarial reviewer in ${ROOT} (Layer 7 build, Wave 2). Do NOT run cargo, pnpm or any build,
do NOT edit any file, do NOT start a process. Review row "| ${id} |" of ${ROOT}/7-relay/TASKS.md (read it whole) against the spec sections it names
(${ROOT}/7-relay/README.md) and the contract (${ROOT}/4-interface/api/openapi.yaml), reading the uncommitted diff of the files the author reports
(cd ${ROOT} && git diff --histogram -- <paths>; a new file: read it whole). Other rows are being built beside it: review only this row's files.
The author's report (its declared deviations are not findings; report what it did not declare): <<<${String(report).slice(0, 9000)}>>>
Hunt for: correctness bugs reachable from a real path (cite file:line now); a criterion unmet or met only by a test helper; the "never adds"
clause broken (a resident task, timer, poll, listener, file write, dependency on an idle or non-hub node: the Raspberry Pi floor); security (the
agent's bearer on a write, a secret logged or framed, foreign bytes trusted, a consent that can be skipped, a size unchecked); contract
mismatches; tests that pass without their fix. CONFIRMED when traced end to end, PLAUSIBLE otherwise. Report only what survives your scrutiny.`,
  { label: `L7:${id}-review`, phase: ph, schema: FINDINGS, model: 'fable' })

const refix = (id, report, rv, ph) => agent(`${CTX}\n\n${T[id].brief}\n\nYou built row ${id}; your report: <<<${String(report).slice(0, 9000)}>>>
An independent reviewer reports: ${JSON.stringify(rv.findings).slice(0, 20000)}
Verify each against the code; fix the real ones, each with a test that fails without its fix; rebut the wrong ones with evidence. Re-run your
row's Check and the gates above. RETURN per finding FIXED / REBUTTED with evidence, the gates with their result lines, files changed.`,
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
const A = await parallel(['D11', 'V1'].filter(want).map(id => () => row(id, 'Stage A')))
log('stage A: ' + A.filter(Boolean).map(r => `${r.id}${r.review ? ' (' + r.review.findings.length + ' findings)' : ''}${r.error ? ' ' + r.error : ''}`).join(' · '))
phase('Stage B')
// L4 and D6d are Rust: one at a time (the box's second cargo slot may be held by another runner); K3 is docs and runs beside them
const B = await parallel([
  () => (async () => { const out = []; for (const id of ['L4', 'D6d'].filter(want)) out.push(await row(id, 'Stage B')); return out })(),
  ...(want('K3') ? [() => row('K3', 'Stage B')] : []),
])
return { A: A.filter(Boolean), B: B.filter(Boolean).flat() }
