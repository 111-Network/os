export const meta = {
  name: 'layer-5-wave-4-review',
  description: 'Layer 5 V4: one fable judge reviews the running system (three-node netns smoke, panel lint, agent token) and the as-built code against the lock criteria; returns majors/minors and a lock verdict',
  phases: [{ title: 'Integrated review', detail: 'fable xhigh, on the running system + the code' }],
}

const ROOT = '/home/kia/projects/111network/os'
const OUT = `${ROOT}/plan/layers/5-messaging/review/integrated.md`

const PROMPT = `
You are the INTEGRATED REVIEWER (V4) for Layer 5 (Messaging & Data) of the 111network OS. Today is 2026-10-19. Repo root:
${ROOT} (git; you NEVER commit). You judge whether Layer 5 may be LOCKED: "no critical or major finding open; every gate green".

THE LAW: ${ROOT}/5-messaging/README.md (spec v1.2 + as-built amendments; §20 lists the lock gates G1–G4; §21 the deferred items) and
${ROOT}/plan/DECISIONS.md D18–D21 (and D23 for what Layer 6 now takes over). The build record: ${ROOT}/5-messaging/TASKS.md (every row,
its readings and bounces), the review triages ${ROOT}/plan/layers/5-messaging/review/{triage,recheck,wave-1,wave-2}.md, and the
coordinates log ${ROOT}/plan/layers/5-messaging/research/coordinates.md (what landed when, what is an external gate).

WHAT RUNS HERE (no hardware; everything in user namespaces): export PATH="$HOME/.cargo/bin:$PATH"; CARGO_BUILD_JOBS=6; one cargo at a time.
1. bash ${ROOT}/4-interface/verify/smoke.sh                 (single node, Layer 4 + 5, ~118 checks)
2. bash ${ROOT}/4-interface/verify/smoke.sh --three-node    (hub + members: cards, direct, hub custody, non-member, replay, restore, requests, agent tools, Meshtastic stub, clean restart; ~154 checks; keep the scratch root with SMOKE_KEEP=1 to inspect logs and stores)
3. bash ${ROOT}/4-interface/verify/panel-lint.sh            (headless Chrome over CDP against the hosted Meshtastic panel; ~144 checks; prints RESIDUAL notes)
4. ${ROOT}/1-bootstrap/scripts/check.sh                     (the full gate; ALL GREEN is the lock condition; ~15 min; run it last)
Never run smoke.sh and panel-lint.sh at the same time (both build ui/dist).

YOUR METHOD: run 1–3 first and read every ok:/FAIL:/note: line as evidence, not as proof: a harness can pass vacuously or assert the wrong
thing. Then review the as-built paths with the eyes of an attacker and of a user on a phone: the sealed envelope and counters as wired in
daemon/fabric/sealer.rs + mesh111 (restore-pending, epochs, attribution, replay), the hub mailbox and custody (one copy, exactly-once
delivery, REJECT_UNKNOWN_DEST honesty), contacts/cards/INTRO requests, the tcp-lan listener exposure (LAN on/off, loopback never a peer,
dial-target refusal), the hosted-panel guards (ticket, cookie, Host/Origin/Fetch-Metadata rules and the documented residual on LAN
origins), the Meshtastic service (meshmud child, shim, presence reader incl. the new keep-alive heartbeat, identify + port-owner table,
untrusted names), the agent tools and the per-spawn bearer token (loopback-only, read-only, proposal-gated send), the installer/updater
Layer 5 paths (verify_layer5_tree call sites, the presence venv build and its fail-closed swap, gen-manifest listing completeness), the
UI (Messages/Contacts/Nearby/Panel tab/Settings copy honesty: states, reasons, untrusted text, restore-pending), and the deviations the
builders recorded in TASKS.md. Check that every §20 gate has either a green run here or is named an external gate honestly in TASKS.md.
Prefer findings you can demonstrate (a request, a test, a log line) over opinions. Do not refactor; do not fix; do not commit.

WRITE ${OUT} (≤ 2500 words): 1. Verdict (lock | fix-then-lock | no-lock) with one paragraph of reasons · 2. Gate runs (the four commands,
their summary lines, every FAIL/note you consider material) · 3. Majors (each: title, where file:line or scenario, how to demonstrate,
the fix in one sentence) · 4. Minors (same shape, short) · 5. Deviations you accept and why (from TASKS.md) · 6. External gates that must
stay listed before the first signed release · 7. What Layer 6 inherits (seams it must not break). Then RETURN the structured verdict.
`

const SCHEMA = {
  type: 'object',
  required: ['verdict', 'gate_runs', 'majors', 'minors', 'external_gates', 'notes'],
  properties: {
    verdict: { type: 'string', enum: ['lock', 'fix-then-lock', 'no-lock'] },
    gate_runs: { type: 'array', items: { type: 'string' }, description: 'one line per command: name, result line, duration' },
    majors: { type: 'array', items: { type: 'object', required: ['title', 'where', 'demonstrate', 'fix'], properties: { title: { type: 'string' }, where: { type: 'string' }, demonstrate: { type: 'string' }, fix: { type: 'string' } } } },
    minors: { type: 'array', items: { type: 'object', required: ['title', 'where', 'fix'], properties: { title: { type: 'string' }, where: { type: 'string' }, fix: { type: 'string' } } } },
    external_gates: { type: 'array', items: { type: 'string' } },
    notes: { type: 'string' },
  },
}

phase('Integrated review')
log('V4: one fable judge on the running system')
const r = await agent(PROMPT, { label: 'judge:integrated', phase: 'Integrated review', schema: SCHEMA, model: 'fable', effort: 'xhigh' })
return r
