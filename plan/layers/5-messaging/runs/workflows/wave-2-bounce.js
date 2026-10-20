export const meta = {
  name: 'layer-5-wave-2-bounce',
  description: 'Layer 5 Wave 2 bounce: three fresh fix agents (B1, U2, F2) apply the review findings per the lead triage on disjoint files; lead re-checks and commits each',
  phases: [{ title: 'Bounce', detail: 'B1 gates · U2 UI · F2 daemon (last: bounce-prone)' }],
}

const ROOT = '/home/kia/projects/111network/os'
const JOURNAL = '/home/kia/.claude/projects/-home-kia-projects-111network/3a9939aa-635d-4885-a9ea-6584b070c677/subagents/workflows/wf_938461a1-1cb/journal.jsonl'
const TRIAGE = `${ROOT}/plan/layers/5-messaging/review/wave-2.md`

const CTX = `
You are a FIX subagent for the 111network OS, Layer 5 (Messaging & Data), Wave 2 bounce. Today is 2026-10-15. Repo root:
${ROOT} (git; you NEVER run git commit/add/stash/checkout — the lead reviews and commits). You will not be messaged
mid-task; everything you need is in this brief and on disk.

YOUR INPUT: the review findings for your task — the line in ${JOURNAL} whose "result.task" starts with your task id (JSON
per line; read it with python3 -I and print majors, minors, deviation_verdicts and notes in full), and the lead's triage
${TRIAGE} — read your section: it carries the lead's decisions where the reviewer left a choice and a few items carried
from other tasks; those decisions are final, implement them. THE LAW stays the locked spec ${ROOT}/5-messaging/README.md
(v1.2 + as-built amendments) and ${ROOT}/plan/DECISIONS.md D19–D21. Fix EVERY major and EVERY minor in your section; for
each, write or extend a test that fails before and passes after where the finding is testable. Keep the task's accepted
readings. If you disagree with a finding, still implement the triage's call and record the disagreement under
"deviations".

FILE OWNERSHIP IS STRICT: only the files your brief lists (plus new files inside those dirs). Two other fix agents run in
parallel on other files, and the Wave 3 verification agents (V1 three-node smoke under 4-interface/verify/**, V2 panel lint)
are running against the committed tree — never touch 4-interface/verify/** or 4-interface/ui/test/tapes/**. A change you need
elsewhere: do not make it, describe it under "deviations".
HOUSE RULES: Rust edition 2021, typed errors, no panics on input, doc comments citing the spec, rustfmt + clippy -D
warnings, no new dependencies; UI: TypeScript strict, Tailwind tokens, D21 budget (256 KB eager + 256 KB lazy, 64 KB per
lazy chunk). Toolchain: export PATH="$HOME/.cargo/bin:$PATH"; a PRIVATE CARGO_TARGET_DIR under /tmp/claude-1000 for every
cargo invocation (two other agents build the same crates). No Go. Never commit.

RUN YOUR CHECK until green; paste its tail. RETURN (structured): task id, status (done | partial | blocked), files
changed, the check command + tail (≤ 25 lines), findings_addressed (one line per finding id: what you did; "carried: …"
for the ones the triage moved), deviations, notes for the reviewer.
`

const TASKS = [
  { id: 'B1', model: 'opus', effort: 'high', brief: `TASK B1-bounce — build/gates (commit 09dc397 + 43c98bd). Findings: 0 majors, 9 minors (gate honesty strings + INTERP/NEEDED + env escape hatches; CC/AR exports in both check.sh files, G2 skip never "ALL GREEN", REQUIRE_G2=1; WIRE verify_layer5_tree at the Layer 1 call sites with tests; gen-manifest cross-checks payloads against its pins; lead decision: unknown pkg/bundle names parse and are ignored, closure stays in verify; lstat every path component; tool-dependent tests fail or skip visibly; notices: SQLCipher LICENSE verbatim + the OpenSSL line, regenerate THIRD-PARTY-NOTICES.md).
Owned: ${ROOT}/1-bootstrap/scripts/{check.sh,cross-build.sh,gen-notices.py}, ${ROOT}/5-messaging/scripts/check.sh (the CC/AR export lines only), 1-bootstrap/crates/node111/src/manifest.rs, 1-bootstrap/crates/node111/src/{update_helper.rs,install/stage.rs,daemon/updater/fetch.rs} (the verify_layer5_tree call sites + their existing tests), 1-bootstrap/crates/node111/tests/manifest_layer5.rs and the Layer 1 updater/installer test files you extend, 1-bootstrap/manifest/gen-manifest.py, 1-bootstrap/README.md (the pkg.* wording only if it needs it), ${ROOT}/THIRD-PARTY-NOTICES.md (regenerated only, via gen-notices.py).
Check: ${ROOT}/1-bootstrap/scripts/check.sh — ALL GREEN (both cross compilers are installed on this box, so G2 must run, not skip); paste the stage lines and the new assert lines.` },
  { id: 'U2', model: 'sonnet', effort: 'high', brief: `TASK U2-bounce — UI Panel tab / Nearby / Settings (commit 282bff8). Findings: 1 major (the page's CSP frame-src is fixed at document load, so a panel enabled afterwards is blocked with no explanation — add the securitypolicyviolation branch with honest copy + reload, warn ahead when the panel was not enabled+listening at the first GET /panels, test it), 11 minors as listed in the triage, plus two carried items: the mock's enroll bundle becomes 111HUB:<card>:<code>:<address> (mock/messaging.ts) and the thread view gains a cancel control for own queued messages calling the new cancelMessage operation (POST /messages/{id}/cancel → 202; F2-bounce adds it to the contract in parallel — code against it defensively: if types.gen.ts has no cancelMessage yet when you typecheck, call it through the adapter with an explicit path and a TODO-free typed wrapper that compiles either way, and say so under deviations). The restore_pending store field is likewise optional until the regen lands.
Owned: ${ROOT}/4-interface/ui/src/views/{services,settings,nearby,messages}/**, src/lib/api/mock/messaging.ts, src/lib/api/{adapter.ts,http.ts,messages.ts,fabric.ts} (additive only: cancel + restore_pending), 4-interface/README.md (the as-built notes), test/** (never test/tapes/**).
Check: cd ${ROOT}/4-interface/ui && pnpm typecheck && pnpm test && pnpm build && pnpm budget.` },
  { id: 'F2', model: 'opus', effort: 'xhigh', brief: `TASK F2-bounce — daemon wiring (commit 7554926). Findings: 1 major (restore-pending turned into a counter reset: shutdown_counters persists announce_last = 0 while id111's CounterStore is restore_pending; guard both call sites; while restore-pending register no tcp-lan / bind nothing / maintain no peers / own_announce empty; AND add the matching guard inside id111's CounterStore (2-identity/crates/id111/src/counters.rs: shutdown/commit/persist refuse while pending, typed error, test, a line in 2-identity/README.md's amendment block); test the sequence end to end), 9 minors as listed in the triage, incl. the contract regen pass you own (offerHub 409, getOwnCard 409, enrollHub 502, decideProposal outcomes, FabricStatus.store.restore_pending, quota-classes description, and the lead's additive cancelMessage: POST /messages/{id}/cancel → 202 for an own queued message via Fabric::cancel, 409 otherwise, reason_code cancelled; pnpm run gen:types), the applied-proposal guard that survives a restart, announce re-mint on a Settings event (additive set_announce on mesh111's TcpLan adapter — you may add that one method in 5-messaging/crates/mesh111/src/link/tcp_lan.rs with a unit test), the restore-pending doc + service-row reason, dial-target validation, synchronous home_hub persistence, settings file moved aside + home_hub rebuilt from the enrolled contact, CLI code from the TTY + byte-safe dechunk, and the three test gaps.
Owned: ${ROOT}/1-bootstrap/crates/node111/src/daemon/fabric/**, src/daemon/dashboard/api_v1/{messages,contacts,fabric,proposals}.rs, src/cli.rs, src/daemon/services/mod.rs (the fabric rule's reason only), tests/api_v1_messages.rs, ${ROOT}/4-interface/api/openapi.yaml + the regenerated 4-interface/ui/src/lib/api/{types.gen.ts,sse.gen.ts} and 4-interface/ui/test/contract.gen.ts (nothing else under 4-interface/ui), ${ROOT}/2-identity/crates/id111/src/counters.rs + its tests + 2-identity/README.md (amendment block line), ${ROOT}/5-messaging/crates/mesh111/src/link/tcp_lan.rs (set_announce only), ${ROOT}/5-messaging/README.md (the §15 as-built note).
Check: ${ROOT}/1-bootstrap/scripts/check.sh cargo stages + ${ROOT}/2-identity/scripts/check.sh (or its cargo test) + ${ROOT}/5-messaging/scripts/check.sh + cd ${ROOT}/4-interface/ui && pnpm run gen:types -- --check && pnpm test — all with private target dirs.` },
]

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

phase('Bounce')
log('Layer 5 Wave 2 bounce: B1 U2 F2 in parallel')
const out = (await parallel(TASKS.map(t => () =>
  agent(CTX + '\n\n' + t.brief, { label: `fix:${t.id}`, phase: 'Bounce', schema: RESULT, model: t.model, effort: t.effort })
    .then(r => r && { ...r, id: t.id })
))).filter(Boolean)
log('returned: ' + out.map(r => `${r.id}=${r.status}`).join(' · '))
return out
