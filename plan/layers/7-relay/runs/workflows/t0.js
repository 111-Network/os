export const meta = {
  name: 'layer-7-t0',
  description: 'Layer 7 T0: one opus agent lays the scaffold the board\'s T0 row names (verify skeleton, module stubs, API router stubs, the ntfy KB candidate entry, the LICENSING fix); the lead verifies and commits',
  phases: [{ title: 'Scaffold', detail: 'one opus agent; compiles, clippy-clean, build.py --check green, shellcheck clean', model: 'opus' }],
}
const ROOT = '/home/user/os'
const SCRATCH = '/tmp/claude-0/-home-user-os/e7852691-6eb6-50f1-9e84-b01d744ea3a9/scratchpad'
const SCHEMA = {
  type: 'object',
  required: ['status', 'files_added', 'files_changed', 'checks', 'deviations', 'notes'],
  properties: {
    status: { type: 'string', enum: ['done', 'partial', 'blocked'] },
    files_added: { type: 'array', items: { type: 'string' } },
    files_changed: { type: 'array', items: { type: 'string' } },
    checks: { type: 'array', items: { type: 'object', required: ['name', 'command', 'result'], properties: { name: { type: 'string' }, command: { type: 'string' }, result: { type: 'string' } } } },
    deviations: { type: 'array', items: { type: 'string' } },
    notes: { type: 'string' },
  },
}
phase('Scaffold')
const r = await agent(`
You build the Layer 7 (Relay & Presence) T0 scaffold of the 111network OS (repo root ${ROOT}). Today is 2026-11-02. Read first, whole:
${ROOT}/7-relay/TASKS.md (the build board; its header paragraph, box rules and the "T0 — lead scaffold" paragraph are your task; the Wave 0 and
Wave 1 rows tell you which files later rows own, so your stubs are exactly the files T0 names and nothing a later row would have to delete),
${ROOT}/7-relay/README.md (the locked spec: §0 for the path abbreviations M/, N/, NI/, ID/, A/, UI/, API, RS/, LC/; §5.9 for the Carrier trait and the
hand-off API shapes; §6.1–§6.2 for the ntfy carrier; §15 for the gates G7–G12 and the harness conventions; §19 item 5 for the LICENSING fix),
${ROOT}/6-transport/verify/lib.sh and one Layer 6 gate script (the REQUIRE_* and SKIPPED convention your stubs follow), ${ROOT}/0-knowledge/AGENTS.md
and ${ROOT}/0-knowledge/README.md (the entry format, the frontmatter fields, the status rubric: a desk-research entry enters as candidate with the
required sections; model your entry on ${ROOT}/0-knowledge/projects/briar-mailbox.md), ${ROOT}/plan/DECISIONS.md D30 Q15–Q16 (what the ntfy entry
must record), ${ROOT}/AGENTS.md (the Pi floor, Linux hygiene).

DO, exactly what the T0 paragraph says:
1. ${ROOT}/7-relay/verify/: lib.sh (sources 6-transport/verify/lib.sh and root.sh; the netns, veth and port names the §15 gates use, as variables),
   and presence.sh (G8), forward.sh (G9), carrier.sh (G10), tls.sh (G11), origin.sh (G12), idle.sh (G7): each a stub with the Layer 6 header comment
   form naming its gate and spec section, that prints "<GATE> SKIPPED: not built yet (T0 stub)" and exits 0 unless REQUIRE_<GATE>=1, then FAIL and
   exit 1 (use lib.sh's skip_or_fail); tapes/.gitkeep and logs/.gitkeep. shellcheck clean (run shellcheck on each; it is installed).
2. node111 module stubs under ${ROOT}/1-bootstrap/crates/node111/src/daemon/: relay/{mod,settings,presence,gateway,netwatch,tls,linkkey,pair,consent,
   metered}.rs; carrier/{mod,ntfy}.rs; origin/mod.rs; registry/mod.rs. Type names only, nothing constructed, no task spawned, no socket, no timer, no
   file read: relay::start(...) returns at once while every switch is off (take the smallest argument set that lets N/mod.rs call it where the other
   subsystems start; read how services/devices start to pick the shape; a stub that takes nothing and returns () is acceptable); relay::tap(event)
   hands each fabric event to presence::on_event and gateway::on_event stubs that do nothing (so the D2 and D3 rows fill only their own files; wire
   relay::tap where fabric events are already fanned out only if that is a one-line, zero-cost hook, else leave a doc comment saying where D2 hooks it).
   carrier/mod.rs: the §5.9/§6.1 Carrier trait with its method signatures as the spec words them (forward_take, forward_done with the
   Taken | Rejected | Retry outcome, carrier_inbound, the pending/target-up event), the Pair and CarrierError types; carrier/ntfy.rs an empty NtfyCarrier
   type with a doc comment pointing at P1. Each stub module starts with a doc comment "Layer 7 (§n): T0 stub; filled by row <id>" and
   #![allow(dead_code)] so that cargo clippy --workspace --all-targets -- -D warnings stays green (the row that fills it removes the allow). pub mod
   lines in daemon/mod.rs; the one relay::start call in N/mod.rs.
3. API router stubs ${ROOT}/1-bootstrap/crates/node111/src/daemon/dashboard/api_v1/{relay7,pair,registry,origins}.rs, each a router() returning
   an empty ApiRouter (or one that answers 404 for its §13.1 paths in the problem+json form the other routers use: read api_v1/mod.rs and one small
   router such as nearby.rs for the ApiRouter type and the mount) merged in api_v1/mod.rs beside the others. Keep the existing api_v1/relay.rs (Layer
   4's agent relay) untouched.
4. ${ROOT}/0-knowledge/projects/ntfy.md: a candidate entry from desk research. Verify the licence from upstream: fetch
   https://raw.githubusercontent.com/binwiederhier/ntfy/main/LICENSE (and the README's licence section) through WebFetch (ToolSearch select:WebFetch;
   raw.githubusercontent.com is allowed by this box's proxy, most sites are denied: mark anything you could not fetch "(unverified)"); record the
   dual licence as read, the latest tag you can establish (the board says v2.36.0 of 2026-10-31; verify from the GitHub tags/releases page if
   reachable or mark unverified), monthly tags, what D30 Q15–Q16 and spec §6.2/§6.4 say the entry must list as unread gates (bus factor, issue
   response, the topic alphabet and length, since=all, the token header, ntfy.sh's limits and retention), that the smoke log (P1) is still to come and
   that only the owner-run G10 log moves it to testing. Then run python3 ${ROOT}/0-knowledge/_meta/build.py --check: 0 errors; fix what it reports
   (the index is rebuilt by build.py: run it as the KB's AGENTS.md says and commit nothing yourself).
5. ${ROOT}/LICENSING.md: the §19 item 5 fix exactly as the spec words it (the Reticulum License line at :56 reads verified; the bao-tree 0.17.0 and
   minisign-verify 0.3.0 notes): read spec §19 item 5 and change only those lines.

RULES: no git write commands (the lead commits); no process left running; cargo only with CARGO_TARGET_DIR=${SCRATCH}/target/node111
CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=3 and TMPDIR unset, one cargo at a time, from ${ROOT}/1-bootstrap; no new dependency; no model names or
attribution lines in any file; nothing resident on a non-hub node (your stubs add no task, socket, timer or file); no change to any file a Wave 0 or
Wave 1 row owns beyond the pub mod / merge lines T0 names. Comments in the stubs cite the spec section they wait for.

CHECKS you run before returning (report each command and its last line verbatim): cd ${ROOT}/1-bootstrap && cargo fmt --all -- --check ·
cargo clippy --workspace --all-targets -- -D warnings · cargo test -p node111 --no-run (a build of the test targets; do not run the full suite) ·
bash -n and shellcheck on every new script · python3 ${ROOT}/0-knowledge/_meta/build.py --check · git status --short (list every file you added or
changed; nothing outside the T0 list). Return: status, files_added, files_changed, checks, deviations, notes.
`, { label: 't0:scaffold', phase: 'Scaffold', schema: SCHEMA, model: 'opus', effort: 'high' })
return r
