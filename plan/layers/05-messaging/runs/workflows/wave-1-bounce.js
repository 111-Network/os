export const meta = {
  name: 'layer-5-wave-1-bounce',
  description: 'Layer 5 Wave 1 bounce: four fresh fix agents (P1, M1, U1, F1) apply the fable/opus review findings on disjoint files; lead re-checks and commits each',
  phases: [{ title: 'Bounce', detail: 'P1 M1 U1 F1 in parallel (F1 last: bounce-prone)' }],
}

const ROOT = '/home/kia/projects/111network/os'
const JOURNAL = '/home/kia/.claude/projects/-home-kia-projects-111network/3a9939aa-635d-4885-a9ea-6584b070c677/subagents/workflows/wf_c857767c-bde/journal.jsonl'
const TRIAGE = `${ROOT}/plan/layers/50-messaging/review/wave-1.md`

const CTX = `
You are a FIX subagent for the 111network OS, Layer 5 (Messaging & Data), Wave 1 bounce. Today is 2026-10-14. Repo root:
${ROOT} (git; you NEVER run git commit/add/stash/checkout — the lead reviews and commits). You will not be messaged
mid-task; everything you need is in this brief and on disk.

YOUR INPUT: the review findings for your task — the line in ${JOURNAL} whose "result.task" starts with your task id
(JSON per line; read it with python3 -I and print majors, minors, deviation_verdicts and notes in full), and the lead's
triage ${TRIAGE} (read your section: it carries the lead's decisions where the reviewer left a choice — those decisions
are final, implement them). THE LAW stays the locked spec ${ROOT}/50-messaging/README.md (v1.2 + as-built amendments) and
${ROOT}/plan/DECISIONS.md D19–D21. Fix EVERY major and EVERY minor in your section unless the triage marks one as
carried to another task; for each, write or extend a test that fails before and passes after where the finding is
testable. Keep the task's recorded, accepted readings; do not re-open them. If you disagree with a finding, still
implement the triage's call and record your disagreement under "deviations".

FILE OWNERSHIP IS STRICT: only the files your brief lists (plus new files inside those dirs). Three other fix agents run
in parallel on other files. A change you need elsewhere: do not make it, describe it under "deviations".
HOUSE RULES: as before — Rust edition 2021, typed errors, no panics on input, doc comments citing the spec, rustfmt +
clippy -D warnings, no new dependencies; UI: TypeScript strict, Tailwind tokens, D21 budget (256 KB eager + 256 KB lazy,
64 KB per lazy chunk). Toolchain: export PATH="$HOME/.cargo/bin:$PATH"; private CARGO_TARGET_DIR under /tmp/claude-1000
for bootstrap builds (another agent builds the same crate). No Go. Never commit.

RUN YOUR CHECK until green; paste its tail. RETURN (structured): task id, status (done | partial | blocked), files
changed, the check command + tail (≤ 25 lines), findings_addressed (one line per finding id: what you did; "carried: …"
for the ones the triage moved), deviations, notes for the reviewer.
`

const TASKS = [
  { id: 'P1', model: 'opus', effort: 'high', brief: `TASK P1-bounce — hosted panels (commit 79ca52d). Findings: 0 majors, 6 minors (m1 Fetch-Metadata on /api paths; m2 bearer callers never mint — lead decision; m3 harness port race; m4 ETag + decoded API-path check; m5 per-request deadline + per-peer-IP cap, and the main dashboard listener's header-read/idle timeouts if contained; m6 random per-generation nonce instead of the pair-code hash).
Owned: ${ROOT}/10-bootstrap/crates/node111/src/daemon/panels/**, src/daemon/dashboard/api_v1/panels.rs, src/daemon/dashboard/routes.rs, src/daemon/dashboard/{mod.rs,bind.rs} (ONLY for the main listener timeouts of m5 — minimal, or skip and record), src/daemon/auth.rs (ONLY if m6's nonce must be written on rotate — minimal), tests/api_v1_panels.rs.
Check: 10-bootstrap/scripts/check.sh cargo stages (fmt, clippy, musl ×2, test) — run with a private CARGO_TARGET_DIR; run tests/api_v1_panels.rs 10× in a loop to prove m3.` },
  { id: 'M1', model: 'opus', effort: 'high', brief: `TASK M1-bounce — Meshtastic (commit 99f5e0a). Findings: 1 major (forget during an in-flight identify persists a binding on a forgotten entry and leaves the service running: order record_identify so published + !forgotten are checked before any store write; on its error identify() tears the service down; test the interleaving), 9 minors (m1 flaky accepted() assert → wait_until; m2 identify step selects on shutdown; m3 retry when the binary appears; m4 zero-length FromRadio frames skipped; m5 session cap atomic; m6 port holder from detail.port_held_by; m7 carried to F2 (openapi text) — do NOT edit openapi.yaml; m8 MeshmudFacts.installed + registry rule "meshmud not installed"; m9 python3-absent flow tests visible).
Owned: ${ROOT}/10-bootstrap/crates/node111/src/daemon/meshtastic/**, src/daemon/devices/{registry.rs,store.rs}, src/daemon/dashboard/api_v1/{devices.rs,nearby.rs}, src/daemon/services/mod.rs (the meshtastic rule only) and src/daemon/services/probe.rs (MeshmudFacts field only), tests/api_v1_meshtastic.rs.
Check: 10-bootstrap/scripts/check.sh cargo stages with a private CARGO_TARGET_DIR; run tests/api_v1_meshtastic.rs 10× under load (e.g. with a parallel cargo build running) to prove m1.` },
  { id: 'U1', model: 'sonnet', effort: 'high', brief: `TASK U1-bounce — UI Messages/Contacts (commit 858077f). Findings: 1 major (ignore semantics — implement the triage's lead decision exactly: ignore = client-side fold with a per-device dismissed set keyed (peer, newest received_seq) that clears on newer activity, no daemon call, request stays pending and leaves the attention count; a separate "forget request" keeps the DELETE behind the 2-step confirm with the honest copy; rewrite the mock + tests), 16 minors as listed (camera leak; stale loadThread responses; page trimming; reuse() keyed; picture pixel cap; reason copy keyed on (state, code); Object.hasOwn tables; path gap recorded + "what this state means:"; badge href → /messages when requests are the only attention items (edit src/app/Header.tsx minimally); agent goal by reason_code; forget copy; add-contact result branches on request; dir="auto" + cleaned raw block; shared Disclosure; card text parse parity (= stripping, 512 + slack); vector tests import encoding.json).
Owned: ${ROOT}/40-interface/ui/src/lib/api/**, src/views/{messages,contacts}/**, src/lib/attention.ts, src/app/Header.tsx (badge href only), test/**. Do not touch prompts.ts, routes, package.json.
Check: cd ${ROOT}/40-interface/ui && pnpm typecheck && pnpm test && pnpm build && pnpm budget.` },
  { id: 'F1', model: 'opus', effort: 'xhigh', brief: `TASK F1-bounce — mesh111 runtime (commit c8a093c). Findings: 2 majors (M1 unbounded actor input queue → bounded, back-pressured frame channel end to end (tcp_lan's bounded channel must not be drained into an unbounded one) selected against the API/event inputs in run(), or try_send + drop + counters.dropped — add a flooding-peer test that proves memory stays bounded; M2 re-enrollment floor → lowest uncommitted custody_seq for that dest − 1 when any is held, else custody_high(); test enrol → hold → re-enrol → fetch delivers), 13 minors as listed by the reviewer and the triage (incl. the REJECTED deviation: node1 pending contacts persisted in a small store table with a schema migration, read in recover(); read marks may stay in memory and must be documented as such; newest-N thread query in the store; lossy test asserting SACK-inferred retransmits before the harness hammer; DUPLICATE-as-relayed reading recorded in the fabric.rs doc; m13 reason codes carried to F2 — do not edit openapi).
Owned: ${ROOT}/50-messaging/crates/mesh111/src/{fabric.rs,frag.rs,outbox.rs,contacts.rs,mailbox.rs,link/**,store/**,lib.rs}, crates/mesh111/tests/**. Do NOT touch the codec files (envelope.rs, ack.rs, custody.rs, ids.rs, enroll.rs, blobs.rs, keys.rs, sealer.rs, test_double.rs) or test-vectors/.
Check: ${ROOT}/50-messaging/scripts/check.sh; run the integration tests 20× in a loop after the channel change (no new flakiness).` },
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
log('Layer 5 Wave 1 bounce: P1 M1 U1 F1 in parallel')
const out = (await parallel(TASKS.map(t => () =>
  agent(CTX + '\n\n' + t.brief, { label: `fix:${t.id}`, phase: 'Bounce', schema: RESULT, model: t.model, effort: t.effort })
    .then(r => r && { ...r, id: t.id })
))).filter(Boolean)
log('returned: ' + out.map(r => `${r.id}=${r.status}`).join(' · '))
return out
