export const meta = {
  name: 'layer-5-wave-2-bounce-2',
  description: 'Layer 5 Wave 2 bounce, second attempt after the 2026-10-15 crash: fresh F2 + U2-mock agents, then B1, over the partial work of WIP commit 6b4f0e5; the lead re-checks and commits each',
  phases: [
    { title: 'Bounce A', detail: 'U2 mock items (sonnet) · F2 daemon wiring resumed (opus xhigh)' },
    { title: 'Bounce B', detail: 'B1 build/gates resumed (opus) after F2 returns — at most two cargo builds overlap on this 12 GB box' },
  ],
}

const ROOT = '/home/kia/projects/111network/os'
const SCRATCH = '/tmp/claude-1000/-home-kia-projects-111network-os/af41ab70-058e-4e3c-bc5e-eec7688f5c7a/scratchpad'
const JOURNAL = `${ROOT}/plan/layers/50-messaging/runs/journals/wave-2-review.wf_938461a1-1cb.jsonl`
const BOUNCE_JOURNAL = `${ROOT}/plan/layers/50-messaging/runs/journals/wave-2-bounce.wf_ba5631d2-319.jsonl`
const TRIAGE = `${ROOT}/plan/layers/50-messaging/review/wave-2.md`
const HANDOFF = `${ROOT}/plan/HANDOFF-2026-10-15.md`

const CTX = `
You are a FIX subagent for the 111network OS, Layer 5 (Messaging & Data), Wave 2 bounce — SECOND ATTEMPT. Today is
2026-10-19. Repo root: ${ROOT} (git; you NEVER run git commit/add/stash/checkout/reset/clean — the lead reviews and
commits). You will not be messaged mid-task; everything you need is in this brief and on disk.

RESUMPTION NOTICE. A previous fix agent for your task was cut off mid-way on 2026-10-15 (the Claude session died). Its
PARTIAL, UNVERIFIED work is on disk: it was committed as-is in the WIP commit 6b4f0e5 ("WIP: Wave 2 bounce (F2, B1) +
Wave 3 (V1, V2) partial work at the 2026-10-15 00:16 crash"). Nothing in your files changed after that commit except a
tree-wide folder rename on 2026-10-18 (bootstrap/ → 10-bootstrap/, identity/ → 20-identity/, interface/ → 40-interface/,
messaging/ → 50-messaging/; "git show --stat 6b4f0e5" and "git diff 43c98bd 6b4f0e5 -- <path>" print the OLD paths, so
query them with the old names) and the relicense commit 7a1d142 (license fields in Cargo.toml). Read ${HANDOFF} §2 (the
per-agent state at the crash) and §5 (lessons). Step 1, INVENTORY: diff 43c98bd..6b4f0e5 for every file you own and list
what the previous agent did, finding by finding. Step 2: KEEP what is correct, FINISH what is incomplete, FIX what is
wrong. Do not start over and do not revert work that is right.

YOUR INPUT: the review findings for your task — the line in ${JOURNAL} whose result.task starts with your task id (JSON per
line; read it with python3 -I and print majors, minors, deviation_verdicts and notes in full) — and the lead's triage
${TRIAGE}, your section: it carries the lead's decisions where the reviewer left a choice and items carried from other
tasks; those decisions are final, implement them. THE LAW stays the locked spec ${ROOT}/50-messaging/README.md (v1.2 +
as-built amendments) and ${ROOT}/plan/DECISIONS.md D19–D21. Fix EVERY major and EVERY minor in your section; for each,
a test that fails before and passes after where the finding is testable. Keep the task's accepted readings. If you
disagree with a finding, still implement the triage's call and record the disagreement under "deviations".

FILE OWNERSHIP IS STRICT: only the files your brief lists (plus new files inside those dirs). Other agents may run in
parallel on other files; a read-only Layer 6 research sweep is reading the tree (ignore it). Never touch
40-interface/verify/** or 40-interface/ui/test/tapes/**. A change you need elsewhere: do not make it; describe it under
"deviations".

HOUSE RULES: Rust edition 2021, typed errors, no panics on input, doc comments citing the spec, rustfmt + clippy -D
warnings, no new dependencies; UI: TypeScript strict, Tailwind tokens, D21 budget (256 KB eager + 256 KB lazy, 64 KB
per lazy chunk). Toolchain: export PATH="$HOME/.cargo/bin:$PATH". THIS BOX HAS 12 GB RAM and killed three sessions
under parallel builds: every cargo invocation uses a PRIVATE target dir under ${SCRATCH}/target-<your task id> AND
CARGO_BUILD_JOBS=6 (export both; never more jobs); never run two cargo commands at once; no cargo clean on other dirs.
No Go. Never commit.

RUN YOUR CHECK until green; paste its tail. RETURN (structured): task id, status (done | partial | blocked), files
changed, the check command + tail (≤ 25 lines), findings_addressed (one line per finding id: what the previous agent
had done, what you did; "carried: …" for the ones the triage moved), deviations, notes for the reviewer.
`

const U2M = { id: 'U2m', model: 'sonnet', effort: 'high', brief: `TASK U2m — the two mock items the U2-bounce could not do for ownership reasons (its report: the line in ${BOUNCE_JOURNAL}; read result.deviations[0] in full — it contains the ready-made wiring). (a) The mock must emit the 111HUB: enroll bundle via hubBundleText() from src/lib/api/mock/messaging.ts (today it still emits 111enroll:); (b) the mock adapter must expose messages.cancel (cancelQueued from mock/messaging.ts) so the cancel control shows in mock mode. Add or extend tests for both. Nothing else: no Rust, no cargo.
Owned: ${ROOT}/40-interface/ui/src/lib/api/mock/index.ts, ${ROOT}/40-interface/ui/test/** (never test/tapes/**).
Check: cd ${ROOT}/40-interface/ui && pnpm typecheck && pnpm test && pnpm build && pnpm budget.` }

const F2 = { id: 'F2', model: 'opus', effort: 'xhigh', brief: `TASK F2-bounce (resumed) — daemon wiring (commit 7554926; partial bounce in 6b4f0e5). Findings: 1 major (restore-pending turned into a counter reset: shutdown_counters persisted announce_last = 0 while id111's CounterStore is restore_pending — guard both call sites; while restore-pending register no tcp-lan / bind nothing / maintain no peers / own_announce empty; AND the matching guard inside id111's CounterStore in 20-identity/crates/id111/src/counters.rs: shutdown/commit/persist refuse while pending, typed error, test, a line in 20-identity/README.md's amendment block; test the sequence end to end), 9 minors as listed in the triage §F2 (dial-target validation in the route and settings::decode; synchronous home_hub persistence; corrupt/oversize settings moved aside + home_hub rebuilt from the enrolled contact; CLI code from the TTY never argv + byte-safe dechunk; the contract regen pass you own — offerHub 409, getOwnCard 409, enrollHub 502, decideProposal outcomes, FabricStatus.store.restore_pending, quota-classes description, the lead's additive cancelMessage POST /messages/{id}/cancel → 202 for an own queued message via Fabric::cancel, 409 otherwise, reason_code cancelled; applied-proposal guard that survives a restart; announce re-mint on a Settings event via an additive set_announce on mesh111's TcpLan adapter; the restore-pending doc sentence + the fabric service-row reason naming the state and which Layer 2 command sets the floor today — if none exists, say so under notes as an owner question; the three test gaps). KNOWN STATE AT THE CRASH (handoff §2): the previous agent's last actions were edits to daemon/fabric/sealer.rs and cargo runs; the contract regen HAS landed (types.gen.ts has cancelMessage and FabricStore.restore_pending; the UI compiles against them). KNOWN REDS TODAY: cargo fmt diffs in 10-bootstrap/crates/node111/src/daemon/dashboard/api_v1/fabric.rs, src/daemon/fabric/cli.rs, tests/api_v1_messages.rs (and one more of your files — run cargo fmt --check to see), and the two restore_pending tests in tests/api_v1_messages.rs fail. Get the WHOLE node111 test suite green (cargo test, not only your tests), clippy -D warnings clean, fmt clean.
Owned: ${ROOT}/10-bootstrap/crates/node111/src/daemon/fabric/**, src/daemon/dashboard/api_v1/{messages,contacts,fabric,proposals}.rs, src/cli.rs, src/daemon/services/mod.rs (the fabric rule's reason only), tests/api_v1_messages.rs, ${ROOT}/40-interface/api/openapi.yaml + the regenerated 40-interface/ui/src/lib/api/{types.gen.ts,sse.gen.ts} and 40-interface/ui/test/contract.gen.ts (nothing else under 40-interface/ui — another agent edits src/lib/api/mock/index.ts and tests in parallel), ${ROOT}/20-identity/crates/id111/src/counters.rs + its tests + 20-identity/README.md (amendment block line), ${ROOT}/50-messaging/crates/mesh111/src/link/tcp_lan.rs (set_announce only), ${ROOT}/50-messaging/README.md (the §15 as-built note).
Check (each with the private target dir + CARGO_BUILD_JOBS=6, one at a time): cd ${ROOT}/10-bootstrap/crates/node111 && cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test; cd ${ROOT}/20-identity/crates/id111 && cargo test; cd ${ROOT}/50-messaging && cargo test --workspace (musl builds are the lead's final gate); cd ${ROOT}/40-interface/ui && pnpm run gen:types -- --check && pnpm test.` }

const B1 = { id: 'B1', model: 'opus', effort: 'high', brief: `TASK B1-bounce (resumed) — build/gates (commit 09dc397 + 43c98bd; partial bounce in 6b4f0e5). Findings: 0 majors, 9 minors per the triage §B1 (gate-honesty asserts on SQLCipher/OpenSSL strings + INTERP/NEEDED on both targets + unset the pkg-config/OPENSSL_NO_VENDOR/OPENSSL_DIR escape hatches; the same CC/AR exports in both check.sh files, messaging's musl block gated on both compilers, a skipped G2 ends "GREEN (G2 SKIPPED — not a lock run)" never "ALL GREEN" and fails under REQUIRE_G2=1; WIRE verify_layer5_tree at the three Layer 1 call sites with tests; gen-manifest cross-checks payloads against its pins; lead decision: unknown pkg/bundle names parse and are ignored, closure stays in verify, manifest-version stays 1; lstat every path component + check panels before hashing + symlinked-bin test; tool-dependent gate tests fail or skip visibly; notices: SQLCipher LICENSE verbatim with "Copyright (c) 2008-2020 Zetetic LLC" + the OpenSSL line, regenerate THIRD-PARTY-NOTICES.md via gen-notices.py). KNOWN STATE AT THE CRASH (handoff §2): the previous agent was near the end, running verification check.sh passes in a scratch copy of the tree; it had added 10-bootstrap/scripts/musl-env.sh + musl-env-test.sh. KNOWN RED TODAY: the hermetic cross-build UI gate 10-bootstrap/scripts/tests/cross-build-ui-gate.sh (run by 10-bootstrap/verify/local-smoke.sh) fails because cross-build.sh sources scripts/musl-env.sh, which that test's synthetic tree never copies — fix it (the test's synthetic tree or a documented fallback in the script), and keep every scripts/tests/* gate honest. The F2 agent has finished (or is finishing) its files; if a check.sh stage fails ONLY because of F2's files (daemon/fabric/**, api_v1/fabric.rs, tests/api_v1_messages.rs, counters.rs), list it as "red: F2 in flight" and do not touch those files.
Owned: ${ROOT}/10-bootstrap/scripts/{check.sh,cross-build.sh,gen-notices.py,musl-env.sh,musl-env-test.sh}, ${ROOT}/10-bootstrap/scripts/tests/** , ${ROOT}/10-bootstrap/verify/local-smoke.sh (only if the gate wiring needs it), ${ROOT}/50-messaging/scripts/check.sh (the CC/AR export lines only), 10-bootstrap/crates/node111/src/manifest.rs, 10-bootstrap/crates/node111/src/{update_helper.rs,install/stage.rs,daemon/updater/fetch.rs} (the verify_layer5_tree call sites + their existing tests), 10-bootstrap/crates/node111/tests/{manifest_layer5.rs,updater_stage.rs,install_flow.rs,helper_guard.rs,common/mod.rs}, 10-bootstrap/manifest/gen-manifest.py, 10-bootstrap/README.md (the pkg.* wording only if it needs it), ${ROOT}/THIRD-PARTY-NOTICES.md (regenerated only, via gen-notices.py).
Check: ${ROOT}/10-bootstrap/scripts/check.sh with CARGO_TARGET_DIR=${SCRATCH}/target-B1 and CARGO_BUILD_JOBS=6 exported — ALL GREEN (both cross compilers are installed, so G2 must run, not skip; any stage red only because of F2's in-flight files is reported as such); plus bash 10-bootstrap/scripts/tests/cross-build-ui-gate.sh green; paste the stage lines and the new assert lines.` }

const RESULT = {
  type: 'object',
  required: ['task', 'status', 'files', 'check', 'findings_addressed', 'deviations', 'notes'],
  properties: {
    task: { type: 'string' },
    status: { type: 'string', enum: ['done', 'partial', 'blocked'] },
    files: { type: 'array', items: { type: 'string' } },
    check: { type: 'string' },
    findings_addressed: { type: 'array', items: { type: 'string' } },
    deviations: { type: 'array', items: { type: 'string' } },
    notes: { type: 'string' },
  },
}

const run = (t, ph) => agent(CTX + '\n\n' + t.brief, { label: `fix:${t.id}`, phase: ph, schema: RESULT, model: t.model, effort: t.effort }).then(r => r && { ...r, id: t.id })

phase('Bounce A')
log('Layer 5 Wave 2 bounce, attempt 2: U2m + F2 in parallel; B1 follows F2 (memory rule)')
const a = (await parallel([() => run(U2M, 'Bounce A'), () => run(F2, 'Bounce A')])).filter(Boolean)
log('A returned: ' + a.map(r => `${r.id}=${r.status}`).join(' · '))

phase('Bounce B')
const b = await run(B1, 'Bounce B')
log('B returned: ' + (b ? `${b.id}=${b.status}` : 'B1=null'))
return [...a, b].filter(Boolean)
