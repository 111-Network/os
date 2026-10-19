export const meta = {
  name: 'layer-5-wave-3-polish',
  description: 'Layer 5 Wave 3 polish: close the gaps the fix wave reported — presence venv wiring + fresh-Debian pip bug (VENVW), enrol-refusal dial leak + counters + layers (F2P), UI stale comments + mock layers (UIP), presence reader keep-alive heartbeat (HB); then gen-manifest listing completeness (MANP)',
  phases: [
    { title: 'Polish A', detail: 'VENVW (opus) ‖ F2P (opus) ‖ UIP (sonnet) ‖ HB (sonnet) — two cargo builds at most' },
    { title: 'Polish B', detail: 'MANP (sonnet) after A: gen-manifest.py listing completeness + its Rust-run tests' },
  ],
}

const ROOT = '/home/kia/projects/111network/os'
const SCRATCH = '/tmp/claude-1000/-home-kia-projects-111network-os/af41ab70-058e-4e3c-bc5e-eec7688f5c7a/scratchpad'
const J = `${ROOT}/plan/layers/50-messaging/runs/journals`

const CTX = `
You are a BUILD/FIX subagent for the 111network OS, Layer 5 (Messaging & Data), Wave 3 polish. Today is 2026-10-19.
Repo root: ${ROOT} (git; you NEVER run git commit/add/stash/checkout/reset/clean — the lead reviews and commits). You
will not be messaged mid-task; everything you need is in this brief and on disk.

STATE: Waves 0–3 are on main through commit 29b4b5b: the fabric, panels, Meshtastic service, UI, gates, the three-node
smoke (154 checks green), panel-lint (144 checks green) and the presence venv builder (10-bootstrap/crates/node111/src/
daemon/transport/venv.rs, module presence). THE LAW is the locked spec ${ROOT}/50-messaging/README.md (v1.2 + as-built
amendments) and ${ROOT}/plan/DECISIONS.md D19–D21. The fix-wave agents' full reports (one JSON line each, read with
python3 -I) are in the harness journal ${ROOT}/../../../.claude/projects/-home-kia-projects-111network-os/af41ab70-058e-4e3c-bc5e-eec7688f5c7a/subagents/workflows/wf_8f3fabca-305/journal.jsonl
(if unreadable, the facts you need are restated in your brief).

FILE OWNERSHIP IS STRICT: only the files your brief lists (plus new files inside those dirs). Other agents run in
parallel on other files. A change you need elsewhere: do not make it; describe it under "deviations".

HOUSE RULES: Rust edition 2021, typed errors, no panics on input, doc comments citing the spec, rustfmt (format only your
own files: rustfmt --edition 2021 <file>, never crate-wide cargo fmt) + clippy -D warnings, no new dependencies; UI:
TypeScript strict, D21 budget; Python: -I everywhere, no network in tests; shell: shellcheck clean. Toolchain: export
PATH="$HOME/.cargo/bin:$PATH". THIS BOX HAS 12 GB RAM: every cargo invocation uses the target dir your brief names AND
CARGO_BUILD_JOBS=6; never two cargo commands at once; no cargo clean elsewhere. Nothing downloads at runtime. Never commit.

RUN YOUR CHECK until green; paste its tail. RETURN (structured): task id, status (done | partial | blocked), files
changed, the check command + tail (≤ 30 lines), findings_addressed, gaps, deviations, notes for the reviewer.
`

const VENVW = { id: 'VENVW', model: 'opus', effort: 'high', brief: `TASK VENVW — wire the presence venv (the VENV agent's two deviations) and fix the fresh-Debian pip gap it found.
(1) Daemon start: in ${ROOT}/10-bootstrap/crates/node111/src/daemon/mod.rs, just before \`let meshtastic = meshtastic::Meshtastic::new(\` (around line 315), run transport::venv::presence::ensure_current(&layout, &crate::util::run::SystemRunner) in tokio::task::spawn_blocking and eprintln the Unavailable error ("111 daemon: presence venv: {error}"); it must complete before Meshtastic::new (spawn_reader checks reader_installed() when the service starts). Test: an integration test (tests/api_v1_meshtastic.rs or a new one) where current/ carries a changed lock → the daemon start rebuilds (fake python per tests/fake_python/mod.rs), and an unchanged lock runs nothing.
(2) Service row reason: MeshmudFacts gains \`reader_unavailable: Option<String>\` (src/daemon/services/mod.rs); Meshtastic::facts() (src/daemon/meshtastic/mod.rs) fills it with \`(!cfg.reader_installed()).then(|| venv::presence::unavailable_reason(&cfg.root).unwrap_or_else(|| "The presence reader is not installed on this node.".into()))\`, cached in refresh() like the other facts; services::meshtastic() gets an arm before \`ChildDisplay::Running if !m.node_db_present\`: Running && !node_db_present && reader_unavailable.is_some() → ServiceState::Error, code "presence_unavailable", reason = that string; add "reader_unavailable" to detail. Registry unit test + an integration test: meshmud running, venvs/presence.status.json says failed → the row is ERROR presence_unavailable with the reason; status ready → unchanged behaviour.
(3) Receipt: in src/install/mod.rs push receipt.degrade_branches when stage::presence_venv returns Outcome::Unavailable (small refactor of step 6; keep it small or report it under deviations).
(4) The rns gap the VENV agent found: packages_for(Family::Debian) (src/install/packages.rs:35-37) installs python3 + python3-venv but not python3-pip, so packages::pip_closure's \`python3 -m pip --version\` fails on a fresh Debian/Ubuntu (confirmed on this box: "/usr/bin/python3: No module named pip"), the rns closure is never prefetched, and the daemon's offline rns venv build fails — a node with no transport. Verify the claim by reading the install flow (packages.rs, daemon/transport/venv.rs ensure/build, install_flow.rs tests), then fix with the smallest correct change (add python3-pip to the Debian set, or download with a build venv's own pip as the presence path does — your call, say why), with a test that fails before and passes after, and one line in 10-bootstrap/README.md where the package set is documented.
Owned: ${ROOT}/10-bootstrap/crates/node111/src/daemon/mod.rs (that block only), src/daemon/services/mod.rs (the meshtastic rule + MeshmudFacts), src/daemon/meshtastic/mod.rs (facts/refresh only), src/install/{mod.rs,packages.rs}, tests/{api_v1_meshtastic.rs,api_v1_status_services.rs,install_flow.rs} and a new test file if cleaner, tests/fake_python/mod.rs (extend only), ${ROOT}/10-bootstrap/README.md (one line).
Check (CARGO_TARGET_DIR=${SCRATCH}/target-VENVW, CARGO_BUILD_JOBS=6): cd ${ROOT}/10-bootstrap/crates/node111 && cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test.` }

const F2P = { id: 'F2P', model: 'opus', effort: 'high', brief: `TASK F2P — three daemon/fabric gaps the three-node smoke found, plus the layer list.
(1) Enrol refusal leaks a dial (V1v gap): after a refused enrolment (a spent code → the hub answers REJECT, the route returns 502), the member keeps dialing the hub (H counted a third attributed peer) and the hub's card stays pinned on the member. mesh111's actor enroll (${ROOT}/50-messaging/crates/mesh111/src/fabric.rs around 3382–3420) calls Dialer::maintain before the hub answers and on_enroll_ok (≈2844) never runs on a refusal. Fix in mesh111: on a refusal or timeout, release the dial unless the hub was already a maintained peer for another reason (home hub / settings peer), and unpin the hub's card if it was pinned only for this attempt (keep it if the contact existed before); typed outcome; tests in 50-messaging/crates/mesh111/tests/hub_flow.rs (refused enrol → no maintained dial, no new pin; a refused re-enrol of an existing member keeps its existing dial) and a node111 integration test in tests/api_v1_messages.rs (502 → the hub's tcp-lan peer count drops back; GET /fabric on the member shows no home hub). The spec's §7.3/§9 wording stays; cite it.
(2) Diagnostics (V1v gap): expose mesh111 FabricStatus.counters (stale, dropped, inferred_lost, … whatever the struct holds) in GET /fabric: node111 src/daemon/fabric/json.rs status() emits a \`counters\` object; ${ROOT}/40-interface/api/openapi.yaml FabricStatus gains \`counters\` (additive, integer fields, described as diagnostics for Advanced); \`pnpm run gen:types\` regenerates 40-interface/ui/src/lib/api/{types.gen.ts,sse.gen.ts} and 40-interface/ui/test/contract.gen.ts; a route test asserts the keys.
(3) Contract wording (V1v gap): openapi Message.reason_code lists hub_not_yours only as "(queued, …)", but spec §9 says the sender keeps \`sent\` with the reason after REJECT_UNKNOWN_DEST, which the daemon emits and the smoke asserts — fix the description only.
(4) Layers: src/daemon/dashboard/api_v1/status.rs layers() (line 242) never reports 5. Add 5 when the fabric is running and not restore-pending (read Facts.fabric / the FabricHandle the status route already has; pick the cleanest source), update the contract's Status.layers description if it enumerates, and test it (api_v1_status_services.rs or dashboard_http.rs): healthy → contains 5; restore-pending or no fabric → does not.
Owned: ${ROOT}/50-messaging/crates/mesh111/src/{fabric.rs,contacts.rs} (the enrol refusal path only) + 50-messaging/crates/mesh111/tests/hub_flow.rs; ${ROOT}/10-bootstrap/crates/node111/src/daemon/fabric/{mod.rs,json.rs}, src/daemon/dashboard/api_v1/{fabric.rs,status.rs}, tests/{api_v1_messages.rs,api_v1_status_services.rs,dashboard_http.rs}; ${ROOT}/40-interface/api/openapi.yaml + the regenerated 40-interface/ui/src/lib/api/{types.gen.ts,sse.gen.ts} and 40-interface/ui/test/contract.gen.ts (nothing else under 40-interface/ui — another agent edits src/lib and test files there); ${ROOT}/50-messaging/README.md (an as-built note line only if needed).
Check (CARGO_TARGET_DIR=${SCRATCH}/target-F2P, CARGO_BUILD_JOBS=6, one cargo at a time): cd ${ROOT}/50-messaging && cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace; cd ${ROOT}/10-bootstrap/crates/node111 && cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test; cd ${ROOT}/40-interface/ui && pnpm run gen:types -- --check && pnpm test.` }

const UIP = { id: 'UIP', model: 'sonnet', effort: 'high', brief: `TASK UIP — three stale spots the DOCS agent found (no Rust, no cargo).
(1) ${ROOT}/40-interface/ui/src/lib/attention.ts lines 6–10: the doc comment says Overview's Needs-you block does not list message requests, but Overview now lists "N message requests" (requestsItem). Make the comment true; keep attentionHref's behaviour (requests-only → /messages) unless a test or the Overview change shows it should now go to /overview#needs-you — decide from the code, say why, and pin it with a test either way.
(2) ${ROOT}/40-interface/ui/src/views/messages/model.ts around line 263: "until U2's messaging.* goals land" is stale (the banner uses the goals); fix the comment only.
(3) ${ROOT}/40-interface/ui/src/lib/api/mock/fixtures.ts:459 reports \`layers: [0, 1, 2, 3, 4]\`. The daemon (status.rs layers()) is being changed in parallel by another agent to report 5 when the fabric runs and is not restore-pending. Make the mock match per scenario: healthy / hub scenarios include 5; the restore-pending scenario and any scenario where the fabric service is not LIVE do not (read src/lib/api/mock/scenarios.ts and messaging.ts for the fabric state per scenario). Adjust tests; add one that pins the per-scenario rule.
Owned: ${ROOT}/40-interface/ui/src/lib/attention.ts, src/lib/api/mock/{fixtures.ts,scenarios.ts} (layers only) and src/lib/api/mock/index.ts (only if the layers value is assembled there), src/views/messages/model.ts (comment only), ${ROOT}/40-interface/ui/test/** (never test/tapes/**, never test/contract.gen.ts).
Check: cd ${ROOT}/40-interface/ui && pnpm typecheck && pnpm test && pnpm build && pnpm budget.` }

const HB = { id: 'HB', model: 'sonnet', effort: 'high', brief: `TASK HB — the presence reader keeps the board's API session alive (Layer 6 research finding, routed to the Layer 5 closure by the critic). Fact, from Meshtastic firmware source v2.7.26 (src/SerialConsole.cpp:27,119-121 and src/mesh/api/ServerAPI.cpp:6,34): the board closes its client-API session after 15 minutes without ANY ToRadio frame (serial and TCP alike); FromRadio stops until a new want_config_id. meshmud (the multiplexer between the board and the reader) forwards every downlink ToRadio verbatim to the board and originates nothing but one want_config at uplink connect (internal/common/uplink.go:95-106). So with no web panel open, the reader's board goes silent after 15 minutes and Nearby freezes. The reader (${ROOT}/50-messaging/sidecars/meshtastic-presence/meshtastic_presence.py) deliberately strips the library's heartbeat ("No send path": the _sendToRadio / _sendToRadioImpl guards refuse every ToRadio but the first want_config; its comment around line 401 says a heartbeat is pointless on a link the board does not own — that reasoning is wrong for the firmware timer, which counts frames the board receives through meshmud). Change: send a ToRadio heartbeat with nonce 0 every 5 minutes (the Rust client and the web SDK do the same; nonce 1 would make the board broadcast NodeInfo on the mesh — a transmit — so the guard must refuse any heartbeat whose nonce is not 0, and refuse every other ToRadio as before). Build the heartbeat ToRadio yourself (the library's own sendHeartbeat / _startHeartbeat names stay banned by the AST tests; extend the allowlist only for the exact helper you add), using a monotonic clock, in the existing loop (no new thread unless the loop needs one). Update the module docstring, the README's "No send path" section ("no send path except the session keep-alive: a heartbeat with nonce 0 every 5 minutes, which transmits nothing on the mesh"), and tests with the fake interface: a heartbeat goes out after the interval and not before; its nonce is 0; a heartbeat with nonce 1 or any other ToRadio is still refused by the guard; the AST tests still pass. One sentence in ${ROOT}/50-messaging/README.md §14's presence-reader bullet. Cite the firmware files in a comment.
Owned: ${ROOT}/50-messaging/sidecars/meshtastic-presence/** (never the lock file or requirements.in), ${ROOT}/50-messaging/README.md (that one sentence).
Check: bash ${ROOT}/50-messaging/sidecars/meshtastic-presence/run-tests.sh (green), and the presence stage of ${ROOT}/10-bootstrap/scripts/check.sh if it runs standalone (read check.sh; if not, say so).` }

const MANP = { id: 'MANP', model: 'sonnet', effort: 'high', brief: `TASK MANP — release-time guard for the payload listing (DOCS agent gap, B1 reviewer deviation 5). update_guard.rs re-verifies the current release at every start with verify_tree, which hashes only the files share/payload.manifest LISTS; an unlisted Layer 5 asset (bin/meshmud, a panels/ file, the reader script) is never re-hashed at boot, and today neither gen-manifest.py nor the node checks listing completeness (the release checklist makes it a manual step). Make ${ROOT}/10-bootstrap/manifest/gen-manifest.py refuse a payload-<triple>.tar.gz whose share/payload.manifest does not list every regular file in the tarball (and lists none that is absent), reading the tarball in memory as it already does for the Layer 5 cross-checks; clear error naming the first offending path. Tests live in Rust: ${ROOT}/10-bootstrap/crates/node111/tests/manifest_layer5.rs runs the generator (generator_refuses_payloads_that_disagree_with_its_pins, generator_round_trip_with_layer5, tests/common/mod.rs build_payload_with) — add cases: an unlisted file is refused; a listed-but-absent file is refused; the round trip still passes. Update the generator's module docstring and the checklist sentence in ${ROOT}/10-bootstrap/docs/release-checklist.md that calls the completeness proof a manual step (it is now enforced by the generator; keep the sha256sum -c line as the human cross-check).
Owned: ${ROOT}/10-bootstrap/manifest/gen-manifest.py, ${ROOT}/10-bootstrap/crates/node111/tests/manifest_layer5.rs, tests/common/mod.rs (extend only), ${ROOT}/10-bootstrap/docs/release-checklist.md (that sentence).
Check (CARGO_TARGET_DIR=${SCRATCH}/target-MANP, CARGO_BUILD_JOBS=6): cd ${ROOT}/10-bootstrap/crates/node111 && cargo test --test manifest_layer5; python3 -I -m py_compile ${ROOT}/10-bootstrap/manifest/gen-manifest.py; shellcheck is not needed.` }

const RESULT = {
  type: 'object',
  required: ['task', 'status', 'files', 'check', 'findings_addressed', 'gaps', 'deviations', 'notes'],
  properties: {
    task: { type: 'string' },
    status: { type: 'string', enum: ['done', 'partial', 'blocked'] },
    files: { type: 'array', items: { type: 'string' } },
    check: { type: 'string' },
    findings_addressed: { type: 'array', items: { type: 'string' } },
    gaps: { type: 'array', items: { type: 'string' } },
    deviations: { type: 'array', items: { type: 'string' } },
    notes: { type: 'string' },
  },
}

const run = (t, ph) => agent(CTX + '\n\n' + t.brief, { label: `fix:${t.id}`, phase: ph, schema: RESULT, model: t.model, effort: t.effort }).then(r => r && { ...r, id: t.id })

phase('Polish A')
log('Polish A: VENVW ‖ F2P ‖ UIP ‖ HB')
const a = (await parallel([() => run(UIP, 'Polish A'), () => run(HB, 'Polish A'), () => run(VENVW, 'Polish A'), () => run(F2P, 'Polish A')])).filter(Boolean)
log('A returned: ' + a.map(r => `${r.id}=${r.status}`).join(' · '))

phase('Polish B')
const b = await run(MANP, 'Polish B')
log('B returned: ' + (b ? `${b.id}=${b.status}` : 'MANP=null'))
return [...a, b].filter(Boolean)
