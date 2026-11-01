export const meta = {
  name: 'layer-6-fix-wave-c',
  description: 'Layer 6 fix wave C: the board Summary follow-ups that need no owner decision (FC1 LXMF parked-send resume, FC2 Delta Chat preflight before files, FC3 405 body + port-owner refresh, FC4 UI network re-read + hidden-tab read, FC5 relay meta observable on DMs)',
  phases: [
    { title: 'Fix', detail: 'opus on Rust, sonnet on the UI; disjoint files; the lead runs the gates and commits' },
  ],
}

const ROOT = '/home/user/os'
const SCRATCH = '/tmp/claude-0/-home-user-os/e7852691-6eb6-50f1-9e84-b01d744ea3a9/scratchpad'

const CTX = `
You are a build agent on the 111network OS (repo ${ROOT}, branch claude/layer-6-implementation-006mte). Layer 6 (Transport &
Bridge) is built and LOCKED as built (D29, 2026-10-23; spec ${ROOT}/6-transport/README.md, board ${ROOT}/6-transport/TASKS.md).
This is fix wave C: the "Open follow-ups" in the board's Summary that need no owner decision. Each task is small, additive
and local; the spec text does not change (you may add an "as built" note to the TASKS.md row your task names, nothing else
in docs). The lead reviews every diff against the task's check, runs the netns gates (G1 bridge.sh, G3 lxmf.sh) and commits;
you commit NOTHING and touch only the files your task owns.

BOX RULES (AGENTS.md "Linux hygiene" + the board's box notes): export CARGO_TARGET_DIR=${SCRATCH}/target/node111
CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=2 for every cargo command (the shared warm target; cargo's lock serializes agents —
wait, never copy the target dir; the disk is a fixed allowance). Unset TMPDIR before tests (a long TMPDIR breaks one devices
test). Run only the crate's unit tests for the modules you touch plus clippy at the end: \`cd ${ROOT}/1-bootstrap && cargo
test -p node111 --lib -- <module path>\` and \`cargo clippy --all-targets -- -D warnings\` (the lead runs the full suite).
The box runs as root: 8 known root-only test failures exist (phone_flow preflight_* ×7, api_v1_messages::
message_proposals_are_checked_then_applied_once) — not yours. Never run 111d, rnsd, the netns harnesses, pnpm dev or any
server yourself; nothing of yours may be left running when you return (ps/ss check). Never add attribution lines anywhere.
Code style: the module docs explain the why in the same voice as the surrounding code; keep edits minimal; no new
dependencies; no new timers, polls, listeners or resident work (the Pi-floor rule).

RETURN (structured): status done|partial|blocked, the files you changed, what you changed in ≤ 10 lines, the exact check
commands you ran with their last line, and any deviation from the brief with the reason.
`

const TASKS = [
  {
    key: 'FC1', model: 'opus', effort: 'high',
    prompt: `TASK FC1 — LXMF: a FAILED while still OUTBOUND must be resumable, not a dead queued row.
File you own: ${ROOT}/1-bootstrap/crates/node111/src/daemon/bridge/lxmf.rs (and its tests in the same file). Read its module
doc (§6 (4) rules), fn state (around line 1444), the heard path that re-hands post-air FAILED sends on the peer's next heard
("resend", around line 1534), and the helper's protocol in ${ROOT}/6-transport/sidecars/lxmf-sidecar/README.md (the state
lines; FA2 added request_path and announce-on-request). The defect (board Summary): "an LXMF FAILED while still OUTBOUND is
reported queued/peer_unannounced but concluded by the helper (nothing resumes it; the row stays queued until the user acts)".
Fix: when the helper reports FAILED for a send that was never on air (OUTBOUND), keep the row queued/peer_unannounced as
today AND register it for resumption exactly like a post-air FAILED: on that destination's next surfaced heard (announce),
re-hand the same text to the helper as a new attempt (the existing resend path), so the row leaves queued without the user
acting. A resumed attempt that fails again parks again (no loop: one attempt per heard). Keep own-echo suppression and the
Accepted-once-on-air rule intact. Add one unit test in the module's existing test style (the monotone rule matrix / f1 tests
are the models) that drives: send → FAILED while OUTBOUND → heard for that peer → a second send line written. Check:
\`cargo test -p node111 --lib -- bridge::lxmf\` green, clippy clean. Note the as-built rule in one sentence in the lxmf.rs
module doc where §6 (4) is explained. Do not touch the Python helper.`,
  },
  {
    key: 'FC2', model: 'opus', effort: 'high',
    prompt: `TASK FC2 — Delta Chat: no files before the component checks.
Files you own: ${ROOT}/1-bootstrap/crates/node111/src/daemon/bridge/deltachat.rs (fn enable, around line 2274; RelayPaths::
prepare and write_madmail_config) and ${ROOT}/1-bootstrap/crates/node111/src/daemon/components/mod.rs (the Components
enable path: enable → enable_with → enable_locked; EnableError::RequiresUnmet / NotCached). Read ${ROOT}/1-bootstrap/crates/
node111/src/daemon/components/mod.rs module doc first. The defect (board Summary): "the madmail prepare hook writes its
first config before the [requires]/payload checks (a refused enable leaves bridge/deltachat/… until 'remove files')". Fix,
additively: give Components a side-effect-free preflight, e.g. \`pub async fn preflight(&self, id: &str) -> Result<(),
EnableError>\` that runs exactly the checks enable_locked runs before it starts anything (the manifest is known, [requires]
glibc/python/service/component, the cache against the pin → NotCached/payload_missing) and returns the same EnableError
variants with the same copy; then in deltachat::enable call preflight for RELAY_COMPONENT (and RPC_COMPONENT) BEFORE
paths.prepare() and write_madmail_config, so a refused enable writes nothing. Keep enable's own checks (idempotent enable;
no double work). Unit tests: one in components (preflight refuses an uncached component with payload_missing and creates
nothing) and one in deltachat's tests if a harness exists there for enable (else a test that the enable order calls
preflight first by observing the directory is absent after a refused enable — follow the existing test style). Check:
\`cargo test -p node111 --lib -- components bridge::deltachat\` green, clippy clean. One-sentence as-built note in the
components module doc.`,
  },
  {
    key: 'FC3', model: 'opus', effort: 'high',
    prompt: `TASK FC3 — two small daemon fixes.
(1) \`POST //api/v1/…\` with a bearer answers 405 with an EMPTY body. Every framework-produced error on /api/* is supposed to
be rewritten to problem+json by routes::api_problems (${ROOT}/1-bootstrap/crates/node111/src/daemon/dashboard/routes.rs
around line 183; read the module doc and api_v1/mod.rs lines 25–45). Find why a doubled slash escapes the rewrite (the
middleware is layered on the /api/* paths — a "//api/v1" path probably does not match the nest and falls through to the
UI fallback or axum's 405 outside the layer) and make it answer problem+json with the same status (405 or 404 — pick what
the contract says for an unknown path; nothing is executed either way). Add an integration test beside the existing ones
in ${ROOT}/1-bootstrap/crates/node111/tests/ (grep for an existing test that asserts problem+json on an extractor rejection
and copy its harness) — if the only harness is heavier than the fix, a unit test on the router is acceptable.
(2) The devices registry's port owners refresh on the 5 s tick, so an upsert right after an identify can answer
409 port_busy once (fails safe). File: ${ROOT}/1-bootstrap/crates/node111/src/daemon/devices/registry.rs (read its module
doc; the PortTable merge around lines 1523–1600; TargetError::Busy; the applier's Refusal::Busy in devices/applier.rs is
a consumer, not yours). Fix without a new timer: when an identify completes (the port claim is released / the holder
changes), refresh the registry's port-owner view immediately from the PortTable (the authority) so the next request sees
the current owner; the 5 s tick stays as it is. Add a unit test in registry.rs's test module in the existing style.
Files you own: routes.rs, registry.rs, one new or extended test file under tests/. Check: \`cargo test -p node111 --lib
-- devices::registry dashboard::routes\` plus the integration test you touched (\`cargo test -p node111 --test <name>\`),
clippy clean.`,
  },
  {
    key: 'FC4', model: 'sonnet', effort: 'high',
    prompt: `TASK FC4 — UI: two follow-ups in the Messages foreign-network store.
Files you own: ${ROOT}/4-interface/ui/src/lib/api/store.ts (networkFrame around line 872, refreshNetworks line 728,
openForeign line 846, the "bridge" case line 474) and ${ROOT}/4-interface/ui/src/views/messages/net/thread.tsx (the read
marking around lines 379–386), plus their tests (grep ${ROOT}/4-interface/ui/src for the existing store tests of
networkFrame / openForeign and extend them in the same style). Read the store's module docs first.
(1) The board Summary: "the UI still reads GET /networks per conversation frame when no list is held (the daemon now
publishes the bridge snapshot on count changes; drop the read)". Verify the daemon side in ${ROOT}/1-bootstrap/crates/
node111/src/daemon/bridge/core.rs lines 530–545 and 820–830 and 956–1010 (the bridge frame is published when the counts
differ, no timer) and the "bridge" SSE case in store.ts; then remove the refreshNetworks() call in networkFrame's
\`conversation && !held\` branch and make the "bridge" frame handler apply the snapshot's networks (counts included) to
state.networks — keep the comment honest (rewrite the networkFrame doc comment's last sentence). Keep the behaviour for a
node older than Layer 6 (networks null).
(2) "a hidden tab is not marked read until the next frame or reload": openForeign reads (and so marks read) only while the
document is visible (line 859). Add a visibilitychange listener registered once by the store (not per view; removed on
dispose/adapter change) that, when the document becomes visible and a foreign conversation is open and has unread > 0 in
the held list (or the page's conversation row says so), re-reads that conversation's newest window (the existing
loadForeign path) so the node counts it read, then refreshes that network's conversations. No polling, no timer.
Check: \`cd ${ROOT}/4-interface/ui && pnpm typecheck && pnpm test\` green (vitest; a network-free run), then \`pnpm build
&& pnpm budget\` (the D21 budget must stay green; report the eager size line). Do not start pnpm dev or any server.`,
  },
  {
    key: 'FC5', model: 'opus', effort: 'high',
    prompt: `TASK FC5 — make the relay meta observable on inbound DMs (contract item).
The board Summary: "not_relayed / meta.relay on an inbound DM are unobservable through the contract (a contract item)".
Trace it: ${ROOT}/1-bootstrap/crates/node111/src/daemon/bridge/ingest.rs (meta.relay admitted around line 160–220),
bridge/edges.rs (meta.not_relayed = "marker" around line 462), the ForeignMessage JSON shaping (grep the daemon for
"not_relayed" and "relayed_from" in dashboard/api_v1/networks.rs and fabric json) and the contract
${ROOT}/4-interface/api/openapi.yaml ForeignMessage (line ~1313; not_relayed is already an additive field at ~1335;
relayed_from is ForeignRelayedFrom). Establish exactly which of the two is dropped for a DM row (a channel row may carry
them) and where — the store (mesh111 store V3 foreign_messages meta column?), the JSON shaping, or the ingest filter —
then make both observable ADDITIVELY: the DM row's JSON carries not_relayed (null when n/a) and a \`relay\` object
mirroring meta.relay ({v, net, id, mid}) under the existing meta/relayed_from shape the contract already has (prefer
filling relayed_from when the semantics match; add a new optional field only if they do not — explain which). Update
openapi.yaml (additive, documented like its neighbours), regenerate the TypeScript (\`cd ${ROOT}/4-interface/ui && pnpm
gen:types\`; the generated file ${ROOT}/4-interface/ui/src/lib/api/types.gen.ts must be committed current — CI checks the
drift) and add a unit test on the Rust side (ingest or the JSON shaping) in the existing style that asserts a DM row shows
the field. Files you own: ingest.rs, edges.rs (only if the fix is there), api_v1/networks.rs JSON shaping (not its router or
networks_view), openapi.yaml (ForeignMessage only), types.gen.ts (regenerated only). Do NOT touch store.ts or thread.tsx
(another agent owns them). Check: \`cargo test -p node111 --lib -- bridge::ingest bridge::edges api_v1::networks\` green,
clippy clean, \`pnpm typecheck\` green after gen:types.`,
  },
]

const RESULT = {
  type: 'object',
  required: ['status', 'files', 'summary', 'checks', 'deviations'],
  properties: {
    status: { type: 'string', enum: ['done', 'partial', 'blocked'] },
    files: { type: 'array', items: { type: 'string' } },
    summary: { type: 'string' },
    checks: { type: 'array', items: { type: 'string' }, description: 'each command with its last line' },
    deviations: { type: 'array', items: { type: 'string' } },
  },
}

const only = (args && args.only) ? String(args.only).split(',') : null
const todo = TASKS.filter(t => !only || only.includes(t.key))
phase('Fix')
log(`fix wave C: ${todo.map(t => t.key).join(', ')} (two slots: Rust tasks interleave with the UI task)`)
// order so that the two slots hold at most two cargo builds at once: FC4 (pnpm) between Rust tasks
const ordered = ['FC1', 'FC4', 'FC2', 'FC3', 'FC5'].map(k => todo.find(t => t.key === k)).filter(Boolean)
const results = await parallel(ordered.map(t => () =>
  agent(CTX + '\n\n' + t.prompt, { label: `fix:${t.key}`, phase: 'Fix', schema: RESULT, model: t.model, effort: t.effort })
    .then(r => r && { key: t.key, ...r })
))
return results.filter(Boolean)
