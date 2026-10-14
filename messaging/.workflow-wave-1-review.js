export const meta = {
  name: 'layer-5-wave-1-review',
  description: 'Layer 5 Wave 1 adversarial review: four independent reviewers over the F1 / P1 / M1 / U1 commits (spec conformance, security, correctness); findings only, no edits',
  phases: [{ title: 'Review', detail: 'F1 fabric · P1 panels · M1 Meshtastic (fable) · U1 UI (opus) — each reads its commit against the locked spec' }],
}

const ROOT = '/home/kia/projects/111network/os'
const JOURNAL = '/home/kia/.claude/projects/-home-kia-projects-111network/3a9939aa-635d-4885-a9ea-6584b070c677/subagents/workflows/wf_f537132f-cdb/journal.jsonl'

const CTX = `
You are an ADVERSARIAL REVIEWER for the 111network OS, Layer 5 (Messaging & Data), build Wave 1. Today is 2026-10-14.
Repo root: ${ROOT} (git; you NEVER edit, stage or commit anything — you return findings; the lead fixes or bounces).
The law is the locked spec ${ROOT}/messaging/README.md (v1.2, D20, with the as-built amendments merged in) and decisions
${ROOT}/plan/DECISIONS.md D18–D20; the task row is in ${ROOT}/messaging/TASKS.md. The builder's own report (status, files,
check tail, recorded deviations, notes) is the line in ${JOURNAL} whose "result.task" starts with your task id — read it
with python3 -I (json per line) and JUDGE EVERY RECORDED DEVIATION: accept (spec-compatible, fail-closed) or reject (spec
violation / unsafe / needs the spec amended first). Then read the commit itself: \`git show --stat <hash>\` and the diff
(\`git show <hash> -- <path>\`), in the builder's suggested reading order, and the tests. Run nothing destructive; you may
run the task's check command and targeted \`cargo test\` / \`pnpm test\` (export PATH="$HOME/.cargo/bin:$PATH"; use a private
CARGO_TARGET_DIR under /tmp/claude-1000 for bootstrap builds).

Hunt for: spec deviations not recorded; fail-open paths; anything that trusts peer-supplied bytes before verification;
unbounded growth (memory, store, file, queue); panics on input (indexing, unwrap, arithmetic overflow, slicing);
wall-clock use in ordering or crypto; TOCTOU on files; secrets in logs/errors; tests that assert the implementation rather
than the spec; "never" claims in docs the code does not keep. Try to construct a concrete failing input or sequence for
every major. A major = spec violation, security hole, data loss, or a crash path; a minor = everything else worth fixing.
Severity honesty: do not inflate; do not pad. If the task is sound, say so.

RETURN (structured): task, verdict (ready | fix-then-lock | not-ready), majors[] and minors[] each {title, where (file:line
or function), why (what breaks, concrete), fix (one line)}, deviation_verdicts[] {deviation (short), verdict accept|reject,
why}, notes (what the lead should read first; open questions for the owner).
`

const TASKS = [
  { id: 'F1', model: 'fable', effort: 'xhigh', brief: `TASK F1 — mesh111 runtime, commit c8a093c (crate ${ROOT}/messaging/crates/mesh111: src/{fabric.rs,frag.rs,outbox.rs,contacts.rs,mailbox.rs,link/**}, tests/**). Lens: protocol + state machine + crypto discipline. Spec §4 (msg_seq, epochs, replay authority A11), §5 (fragments, SACK, ≤16 outstanding, eviction), §6 (ACK ordering, custody receipts), §7 (cards, INTRO tcp-lan-only, requests), §8/§8.1 (links, attribution after a pair_mac-verified Fresh packet), §9 (mailbox roles, states, never-abandoned n), §18 (verify-before-state; no timers; no wall clock). Specific checks: (1) attribution can never be obtained by a replayed or unverified packet — trace the code path; (2) a hub never pushes to an unattributed connection; (3) the evicted-partial SACK handling (both halves) cannot lose a message or resend forever; (4) ACK wire order is C2's canonical order AND the §6.1 selection priority holds — construct the >8 ranges case; (5) the RestorePending reading for send(); (6) custody reject → state/reason mapping vs §9 copy and the C1 contract enums (not_enrolled is not in the contract — is that a spec gap or a builder error?); (7) in-memory node1 contacts and read marks — what is lost on restart and is that acceptable for v1 or a C3 column now?; (8) tcp_lan.rs: length caps before allocation, hello/announce-first enforcement, back-off without timers (how?), listener never bound by the crate; (9) tests actually exercise the lossy path at 160 B with loss+reorder. Check command: ${ROOT}/messaging/scripts/check.sh.` },
  { id: 'P1', model: 'fable', effort: 'xhigh', brief: `TASK P1 — hosted panels in node111, commit 79ca52d (${ROOT}/bootstrap/crates/node111/src/daemon/panels/**, dashboard/api_v1/panels.rs, dashboard/routes.rs, tests/api_v1_panels.rs). Lens: web security of a multi-origin host behind one daemon. Spec §13 as built in v1 (static panels + built-in shims; generic proxy reserved) and D19(d)/D20 panel deltas (guards on every panel request, CORS stripped, no loopback ticket skip, 308 localhost → 127.0.0.1, ticket → p111_<id> cookie). Specific checks: (1) can a page on another origin (incl. another panel's origin, or the dashboard itself) read or drive a panel's /api/v1/* — walk Host / Origin / Sec-Fetch-Site / cookie SameSite=Strict / CORS stripping / OPTIONS refusal; (2) ticket lifecycle: single use, 30 s, bound to the minting session, spent on any method, never logged, query redaction actually applied where URIs are logged (builder says no URI is logged — verify, incl. the main dashboard's logging of the mint route); (3) session file 0600 + atomic writes + revocation on rotate/disable/LAN-off/logout — any window where a revoked cookie still works?; (4) static serving: path traversal (.., encoded, symlinks), SPA fallback never under /api, ETag/hash of whole file (DoS with a 64 MiB file?), MIME sniffing; (5) listener: header-read + idle timeouts, connection cap, request body cap, slowloris; (6) per-request dashboard CSP and frame-ancestors on panel responses — can a panel be framed by a non-dashboard origin on LAN?; (7) the 308 rule exemptions (/health, /api/status) — safe?; (8) Termux (untrusted loopback) still needs the ticket — verify the branch; (9) the agent's bearer token on trusted loopback can mint panel tickets (builder's open question) — is that acceptable under D20 (agent bearer loopback-only, /api/v1 agent-facing)? Check command: bootstrap/scripts/check.sh cargo stages (UI stage is red on the bundle budget — known).` },
  { id: 'M1', model: 'fable', effort: 'xhigh', brief: `TASK M1 — Meshtastic in node111, commit 99f5e0a (${ROOT}/bootstrap/crates/node111/src/daemon/meshtastic/**, dashboard/api_v1/{nearby.rs,devices.rs}, devices/{registry.rs,store.rs}, daemon/mod.rs wiring, tests/api_v1_meshtastic.rs). Lens: process supervision, untrusted input ingest, port ownership, and the GPL boundary (D19(g): meshmud MIT, the Python presence reader GPL — nothing GPL links into node111; protobuf decoding never enters our binary). Specific checks: (1) identify flow: consent required, PortTable claim → identify child → --once → transfer → service child in place, never two opens of the port, and what happens on every failure branch (reader exit 3, meshmud crash, client disconnect mid-probe — builder spawns a task; is the claim always released?); (2) config rendering: serial path regex anchored, no shell, file 0600, TCP binding only loopback downlink, advertise=false — any injection via devices.json fields into the TOML?; (3) presence ingest hardening: O_NOFOLLOW, regular file ≤ 1 MiB, schema 1, ≤ 4096 nodes, names ≤ 40 bytes (byte cut on a UTF-8 boundary?), typed fields, invalid ⇒ keep last + counter — construct a hostile file; (4) the shim: one TCP downlink per panel session opened on first /api/v1 call, framing 0x94 0xC3 len strict (max frame size? partial reads?), bounded drop-oldest with drop count, idle close, revocation closes, session cap 16 — any way a panel session can reach meshmud without a valid p111 cookie or share another session's queue?; (5) 'one frame per /fromradio call' deviation vs the official web client's transport-http expectations (all=true concatenation) — does the official client cope?; (6) registry/store changes: binding retained for absent devices, revalidated on load, one binding only — any way an attacker-written devices.json path reaches meshmud's serial open?; (7) restart ordering and shutdown joins; (8) the identify restart policy 'first exit final' and the swap — can the monitor respawn a crashed identify child and open the port twice?; (9) test flakiness the P1 builder observed in shim_round_trip_buffer_and_session_lifecycle (2/4 failures under load) — find the race. Check command: bootstrap/scripts/check.sh cargo stages.` },
  { id: 'U1', model: 'opus', effort: 'high', brief: `TASK U1 — UI data layer + Messages + Contacts, commit 858077f (${ROOT}/interface/ui: src/lib/api/**, src/views/{messages,contacts}/**, src/lib/attention.ts, routes/nav, test/**). Lens: contract conformance (interface/api/openapi.yaml as regenerated by C1), honesty of copy (spec §9 state sentences, §15/§16 rules: received vs sender-claimed time, requests never auto-accepted, peer text never interpreted), XSS/injection via peer-supplied strings (aliases, labels, card text, QR payloads) into DOM/URLs/prompts, and the DESIGN.md/mode rules the other views follow. Specific checks: (1) every peer-supplied string path to the DOM — React escaping is not enough for href/src/attribute sinks and for the AgentButton prompt text (builder says no peer text enters a prompt — verify); (2) card parsing: bech32 node1 validation against identity/test-vectors/encoding.json incl. negatives, size caps on pasted text and QR payloads, no exceptions on garbage; (3) QR scan: getUserMedia only on secure/loopback origins (how is 'loopback' decided — isSecureContext? hostname list?), jsQR lazy import only on first scan, file fallback, canvas own-card drawing from the {size, rows} bitmap; (4) state matrix copy vs §9 exactly (queued/sent/relayed/delivered + reasons waiting_for_key, hub_not_yours, restore_pending, mailbox_full, store_full) and 'ignore' = DELETE behind confirm vs §7.2 'kept in the request list' (builder's open question — decide which reading the spec requires); (5) store applyFrame for message/contact/fabric/nearby/panels frames — ordering, dedup, unread counts, requests count in attention; (6) the mock scenarios validate against the contract (run pnpm test) and the fixtures' ports match the spec table; (7) bundle: what exactly is eager vs lazy (routes, views, jsQR) — give the lead the cheapest honest path under the 200 KB gate without breaking the other views (do NOT change anything). Check: cd ${ROOT}/interface/ui && pnpm typecheck && pnpm test && pnpm build (budget is known red).` },
]

const RESULT = {
  type: 'object',
  required: ['task', 'verdict', 'majors', 'minors', 'deviation_verdicts', 'notes'],
  properties: {
    task: { type: 'string' },
    verdict: { type: 'string', enum: ['ready', 'fix-then-lock', 'not-ready'] },
    majors: { type: 'array', items: { type: 'object', required: ['title', 'where', 'why', 'fix'], properties: { title: { type: 'string' }, where: { type: 'string' }, why: { type: 'string' }, fix: { type: 'string' } } } },
    minors: { type: 'array', items: { type: 'object', required: ['title', 'where', 'why', 'fix'], properties: { title: { type: 'string' }, where: { type: 'string' }, why: { type: 'string' }, fix: { type: 'string' } } } },
    deviation_verdicts: { type: 'array', items: { type: 'object', required: ['deviation', 'verdict', 'why'], properties: { deviation: { type: 'string' }, verdict: { type: 'string', enum: ['accept', 'reject'] }, why: { type: 'string' } } } },
    notes: { type: 'string' },
  },
}

phase('Review')
log('Layer 5 Wave 1 review: F1 P1 M1 (fable) · U1 (opus), in parallel')
const out = (await parallel(TASKS.map(t => () =>
  agent(CTX + '\n\n' + t.brief, { label: `review:${t.id}`, phase: 'Review', schema: RESULT, model: t.model, effort: t.effort })
    .then(r => r && { ...r, id: t.id })
))).filter(Boolean)
log('verdicts: ' + out.map(r => `${r.id}=${r.verdict} (${r.majors.length} major / ${r.minors.length} minor)`).join(' · '))
return out
