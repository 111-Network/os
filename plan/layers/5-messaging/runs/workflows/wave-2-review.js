export const meta = {
  name: 'layer-5-wave-2-review',
  description: 'Layer 5 Wave 2 adversarial review: three independent reviewers over the B1 / U2 / F2 commits (spec conformance, security, correctness); findings only, no edits',
  phases: [{ title: 'Review', detail: 'B1 gates (opus) · U2 UI (opus) · F2 daemon wiring (fable) — each reads its commit against the locked spec' }],
}

const ROOT = '/home/kia/projects/111network/os'
const JOURNAL = '/home/kia/.claude/projects/-home-kia-projects-111network/3a9939aa-635d-4885-a9ea-6584b070c677/subagents/workflows/wf_97dba36d-e98/journal.jsonl'

const CTX = `
You are an ADVERSARIAL REVIEWER for the 111network OS, Layer 5 (Messaging & Data), build Wave 2. Today is 2026-10-14.
Repo root: ${ROOT} (git; you NEVER edit, stage or commit anything — you return findings; the lead fixes or bounces).
The law is the locked spec ${ROOT}/5-messaging/README.md (v1.2, D20, with the as-built amendments merged in), decisions
${ROOT}/plan/DECISIONS.md D18–D21, and the task row in ${ROOT}/5-messaging/TASKS.md (its status cell names the commit and
the builder's recorded readings). The builder's own report (files, check tail, recorded deviations, notes) is the line in
${JOURNAL} whose "result.task" starts with your task id — read it with python3 -I (JSON per line) and JUDGE EVERY RECORDED
DEVIATION: accept (spec-compatible, fail-closed) or reject (spec violation / unsafe / needs the spec amended first). The
Wave 1 triage ${ROOT}/plan/layers/5-messaging/review/wave-1.md records the lead's standing calls (ignore = client-side fold, bearer
never mints panel tickets, contract additions carried to F2) — do not re-open them. Then read the commit: \`git show
--stat <hash>\` and the diff per file in the builder's suggested order, and the tests. You may run the task's check and
targeted tests (export PATH="$HOME/.cargo/bin:$PATH"; private CARGO_TARGET_DIR under /tmp/claude-1000 for bootstrap).

Hunt for: spec deviations not recorded; fail-open paths; peer- or file-supplied bytes trusted before verification;
unbounded growth; panics on input; wall-clock use in ordering or crypto; TOCTOU; secrets in logs/errors/state files;
tests that assert the implementation rather than the spec; "never" claims the code does not keep. Construct a concrete
failing input or sequence for every major. Major = spec violation, security hole, data loss, crash path; minor = worth
fixing. Severity honesty: do not inflate; do not pad. If the task is sound, say so.

RETURN (structured): task, verdict (ready | fix-then-lock | not-ready), majors[] and minors[] each {title, where, why,
fix}, deviation_verdicts[] {deviation, verdict accept|reject, why}, notes (what the lead should read first; open
questions for the owner).
`

const TASKS = [
  { id: 'B1', model: 'opus', effort: 'high', brief: `TASK B1 — build/gates plumbing, commit 09dc397 (1-bootstrap/scripts/{check.sh,cross-build.sh}, 1-bootstrap/verify/local-smoke.sh, crates/node111/src/manifest.rs + tests/manifest_layer5.rs, 1-bootstrap/manifest/gen-manifest.py, 1-bootstrap/TASKS.md sign-off). Lens: supply-chain and gate honesty (spec §19 L1, §20 G2; D19 e/g, D20). Checks: (1) every new check.sh stage really fails on the failure it claims to catch (SQLCipher not linked, a tampered bundle tree, a wrong lock hash, a missing cross compiler) — read the assertions, don't trust the echo lines; (2) manifest.rs: deny_unknown_fields kept, closed names enforced on parse not only on verify, verify_layer5_tree cannot be bypassed by a symlink/hardlink/mode trick inside the payload tree, size + sha256 + tree-sha256 all checked, no path traversal in bundle entries; the builder's deviation 4 says the three updater call sites are NOT wired — confirm what protects the Layer 5 assets today (payload tarball pin + share/payload.manifest) and whether that is sufficient until the call sites land; (3) the dependency sign-off table vs Cargo.lock as of HEAD: every crate mesh111 pulls into node111 is listed with the right license (spot-check 8, incl. foldhash Zlib, SQLCipher BSD-3, OpenSSL Apache-2.0), nothing GPL/AGPL/non-OSI entered; (4) cross-build.sh static assertions (no dynamic libcrypto/libsqlcipher), reproducibility of the tree hash vs meshtastic-web-build.sh; (5) gen-notices.py: THIRD-PARTY-NOTICES is stale on HEAD and OpenSSL 3.6.3 is unnamed (builder's deviation 7) — judge whether shipping notices without the OpenSSL text is a licence problem and say exactly what text must appear. Check: 1-bootstrap/scripts/check.sh (expect it to pass or fail only on the stale notices).` },
  { id: 'U2', model: 'opus', effort: 'high', brief: `TASK U2 — UI Panel tab, Nearby, Settings L5, service pages, prompts, commit 282bff8 (4-interface/ui: src/views/{services,nearby,settings,overview}/**, src/lib/{prompts,layers,nav,verdict}.ts, views/messages/banner.tsx, test/**; 4-interface/README.md). Lens: contract conformance (openapi.yaml at HEAD, incl. F2's additions if present), the §13 dashboard-side rules for hosted panels, and injection via peer-/panel-supplied strings. Checks: (1) ticketSrc fail-closed: enumerate every way the mint reply's url could point the iframe at another origin, scheme, port, credentials, or a data:/javascript: URL — and confirm each is refused; the ticket never reaches a log, the URL bar of the parent, localStorage, or a prompt; Reload/Open-full-client each mint a fresh ticket and never reuse one; (2) iframe attributes: sandbox exactly as the manifest's 'default' means in P1's panels/manifest.rs, allow built only from validated feature names, referrerpolicy, no allow-same-origin+allow-scripts pair unless the spec requires it (it does: the panel is its own origin — verify the reasoning and that the frame can never be same-origin with the dashboard); (3) Nearby: names/ids/positions cleaned and cut, dir=auto + isolation, never into a link, prompt, or title attribute; stale/absent source copy honest; (4) Settings: no optimistic state; the one-time enroll code is shown once and never persisted; PATCH bodies match the contract; the 111HUB bundle parser matches mesh111::contacts::HubBundle (read the crate) and refuses hostile input; store_full / restore pending copy per §9/§10; (5) prompts: no peer text, panel.explain and meshtastic.setup goals name only daemon facts; existing prompts byte-identical (test pins); (6) the builder's readings: /settings/l5 in Simple mode (IA), no Identify button, loopback-only handled reactively — accept or reject; (7) tests: do the 1065 cover the fail-closed branches above, or only happy paths? Check: cd 4-interface/ui && pnpm typecheck && pnpm test && pnpm build && pnpm budget.` },
  { id: 'F2', model: 'fable', effort: 'xhigh', brief: `TASK F2 — daemon wiring + routes, the commit named in the F2 row of 5-messaging/TASKS.md (1-bootstrap/crates/node111/src/daemon/fabric/** incl. sealer.rs, dashboard/api_v1/{messages,contacts,fabric,proposals}.rs, daemon/mod.rs hooks, cli.rs, Cargo.toml mesh111 dependency, tests/api_v1_messages.rs, tests/README.md; 4-interface/api/openapi.yaml additions + regenerated types; 5-messaging/README.md §15 note). Lens: the Sealer over id111 is where key material meets the fabric — review it as crypto code; the routes are the only surface the UI and the agent see. Checks: (1) sealer.rs: pair-key cache zeroized on forget and on drop; seal_payload calls the inner closure exactly once with the real epk and AEADs exactly what it returns; open_payload returns the AD-verified header and the packet's epk; sign prepends the ASCII domain tag; observe_counter returns the A11 verdict and advances high-water either way (never enforces the window for msg_seq kinds); next_announce_counter commits before returning and errors under RestoreFloorRequired; store_secret = BLAKE2b-256(key = X25519 private key bytes 32..64, data "111/store-secret/v1") computed without the private key leaving the handle and zeroized; random from the §4 entropy discipline (no fallback); no key bytes in Debug/logs/errors; (2) store open: Mode S key from the handle, fabric/ dirs 0700, settings 0600, DiskSpace source, complete_partials at start, clean flag on shutdown, the daemon never holds the store key in a long-lived String; (3) listener binding only when FabricSettings.listen_lan / hub_role say so and only on the LAN address the dashboard uses; dialer targets validated (host:port, no DNS surprises, no loopback-to-self loops); (4) routes vs the contract: auth matrix (dashboard session / agent bearer read-only / nothing), 202 vs 409 restore_pending, loopback-only on settings/offer/enroll/send for the bearer, Problem codes, SSE coalescing per message per tick, Last-Event-ID replay unaffected; (5) proposal acceptance: parses the A1 body contract strictly (64 lowercase hex, exactly one blank line, text verbatim), checks the node-id stamp equals this node, recipient is an accepted contact (never a pending request), marks applied only after the store commits — construct a malformed proposal file that must be refused; (6) CLI 111 fabric offer|enroll|status: trusted-loopback only, no secrets on argv, the enroll address/code validation; (7) the openapi additions are additive and regenerated types/digest match (pnpm run gen:types -- --check); (8) the builder's recorded deviations and the two readings carried from Wave 1 (RestorePending ⇒ 409, not_enrolled/cancelled codes); (9) tests: does api_v1_messages.rs run a real fabric with a real id111 keyset or a TestSealer — and does it exercise the auth matrix, the 409, proposal refusals, SSE coalescing, listener gating? Check: 1-bootstrap/scripts/check.sh cargo stages (private CARGO_TARGET_DIR) + cd 4-interface/ui && pnpm run gen:types -- --check && pnpm test.` },
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
log('Layer 5 Wave 2 review: B1 U2 (opus) · F2 (fable), in parallel')
const out = (await parallel(TASKS.map(t => () =>
  agent(CTX + '\n\n' + t.brief, { label: `review:${t.id}`, phase: 'Review', schema: RESULT, model: t.model, effort: t.effort })
    .then(r => r && { ...r, id: t.id })
))).filter(Boolean)
log('verdicts: ' + out.map(r => `${r.id}=${r.verdict} (${r.majors.length} major / ${r.minors.length} minor)`).join(' · '))
return out
