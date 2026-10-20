export const meta = {
  name: 'layer-5-wave-2',
  description: 'Layer 5 build Wave 2: B1 build/gates plumbing, U2 UI Panel tab + Nearby + Settings + prompts, F2 daemon wiring + routes + Sealer over id111 — 3 parallel tasks on disjoint files',
  phases: [{ title: 'Wave 2', detail: 'B1 U2 F2 in parallel (F2 last in the list: bounce-prone); lead reviews + commits each' }],
}

const ROOT = '/home/kia/projects/111network/os'

const CTX = `
You are a BUILD subagent for the 111network OS, Layer 5 (Messaging & Data). Today is 2026-10-14. Repo root: ${ROOT}
(a git repo; you NEVER run git commit/add/stash/checkout — the lead reviews and commits each task). You will not be
messaged mid-task; everything you need is in this brief and on disk.

THE LAW: the locked spec ${ROOT}/5-messaging/README.md (v1.2, D20, plus the as-built amendments merged into it) and your
task row in ${ROOT}/5-messaging/TASKS.md. Read your row first, then the spec sections the row names, then the landed code
you build on (named in your brief). Decisions: ${ROOT}/plan/DECISIONS.md D19, D20, D21. Do not re-open decisions;
implement them. If the spec is ambiguous, pick the reading that is safest (fail closed) and record it under "deviations".

WHAT EXISTS (Waves 0–1, all committed on main): mesh111 crate at 5-messaging/crates/mesh111 (codec, SQLCipher store, blobs,
runtime: src/fabric.rs \`Fabric::start\` actor with a typed API + \`FabricEvent\` subscription + \`link_events\` sink +
\`gate\`; link/tcp_lan.rs adapter with \`TcpLan::listen\`/dial; frag/outbox/contacts/mailbox); node111 (bootstrap crate):
daemon/panels/** (PanelHost, tickets, listeners, guards, shims), daemon/meshtastic/** (meshmud + presence children,
identify, Nearby, meshtastic-http shim), daemon/ports.rs PortTable, services registry rules for \`fabric\`/\`meshtastic\`
(services/mod.rs) with \`Facts.fabric\` / \`Facts.meshmud\` providers (services/probe.rs), auth::DaemonToken + the
AGENT111_DAEMON_TOKEN / AGENT111_DAEMON_API_URL env for the agent, api_v1/{panels,nearby,devices}.rs; the contract
4-interface/api/openapi.yaml with 19 Layer 5 operations (C1) + regenerated types; agent111 tools (A1) incl. the
\`messages.send\` proposal body contract; UI data layer + Messages + Contacts views (U1) with slices for
messages/contacts/fabric/nearby/panels and the D21 budget gate (256 KB eager + 256 KB lazy, 64 KB per lazy chunk).
Seam map for node111: ${ROOT}/plan/layers/5-messaging/research/00-code-seams.md. The Wave 1 builders' recorded deviations and the
fable review findings are summarised in ${ROOT}/5-messaging/TASKS.md rows and ${ROOT}/plan/layers/5-messaging/review/wave-1.md —
read your task's section before you start.

FILE OWNERSHIP IS STRICT: create or modify ONLY the files/dirs your row lists under "Files (owned)" plus what your brief
adds (and new files inside those dirs). Two other tasks run in parallel on other files right now; if you need a change
elsewhere, do NOT make it — describe it under "deviations" and continue (exception: a tiny additive \`pub mod\` / route
registration / Cargo dependency line a new module needs — do it and list it).

HOUSE RULES: Rust — edition 2021, typed errors (never panic on input; sizes checked before parse), doc comments citing
the spec section, tests, rustfmt + clippy -D warnings clean; node111's dependency policy is ${ROOT}/1-bootstrap/TASKS.md
"Dependency policy" — the \`mesh111\` path dependency (and what it pulls: rusqlite+SQLCipher, bao-tree, blake3, ciborium,
blake2, chacha20poly1305) is pre-approved by D19/D20; anything else new: do not add it, report it. UI — TypeScript
strict, Tailwind 4 tokens only, DESIGN.md rules as the existing views do, no new npm deps, D21 budget. Nothing downloads
at runtime. No Reticulum imports. No wall clocks in crypto/ordering. Never commit.
Toolchain: export PATH="$HOME/.cargo/bin:$PATH"; musl cross toolchains at ~/.local/toolchains/*/bin (the check scripts
set them up; 5-messaging/scripts/check.sh shows the SQLCipher/openssl-src env that works for both targets); node/pnpm
present; Python 3.10; no Go. Use a private CARGO_TARGET_DIR under /tmp/claude-1000 for bootstrap builds (another task
builds the same crate in parallel).

RUN YOUR CHECK (the row's "Check" column) until it is green; paste its tail. A check that fails only because of a file
another task owns: wait 60 s, retry twice, then report "blocked-by: <task>" — never touch that file.

RETURN (structured): task id, status (done | partial | blocked), files changed/created (exact paths), the check
command + its tail (≤ 25 lines), deviations (each with why), notes for the reviewer (what to read first, open
questions). Factual; the lead reads diffs.
`

const TASKS = [
  { id: 'F2', model: 'opus', effort: 'xhigh', brief: `RESUMPTION NOTICE: a previous instance of you worked on this task for about 1 h 45 min and was cut off by a session crash before it could return. Its partial work is ON DISK, uncommitted (git status --short / git diff --stat show it; B1 and U2 are committed and gone from this run): src/daemon/fabric/{mod,sealer,settings,json,times,proposal,cli}.rs, api_v1/{messages,contacts,fabric}.rs, proposals.rs (message kind), cli.rs, daemon/mod.rs hooks, node111 Cargo.toml + the workspace Cargo.lock with the mesh111 path dependency, 4-interface/api/openapi.yaml + the regenerated types.gen.ts / sse.gen.ts / contract.gen.ts, a 5-messaging/README.md §15 as-built note, tests/README.md and tests/api_v1_messages.rs (1413 lines). At the cut it was fixing COMPILE ERRORS in tests/api_v1_messages.rs (a message_frames helper lifetime, among others); the check had NOT been green yet and its scratch log is gone. First inventory the work (read every file above), build with cargo check --tests, keep what is correct, finish the rest; do NOT rewrite from scratch what exists and compiles. Note from B1 (landed): check.sh now runs the mesh111 stages and asserts SQLCipher is linked into node111 whenever Cargo.lock lists mesh111, so your binary must really open the store; THIRD-PARTY-NOTICES.md is stale against your Cargo.lock — do NOT regenerate it (the lead does). Note from U2 (landed): the Settings page parses the enroll bundle as 111HUB:<card>:<code>:<address> (mesh111::contacts::HubBundle) and sends enroll {address: host:port}; your route must accept a hostname or IP literal with a port, and offer must answer 409 with an honest detail when the node is not a hub. TASK F2 — daemon wiring + routes (spec §2, §9, §15, §17; §10.2 Mode S; §4 epoch source). Read mesh111's src/fabric.rs (Fabric::start, FabricSettings, SettingsPatch, FabricEvent, the typed API, link_events/gate), src/sealer.rs (the Sealer trait: seal_payload with the epk closure, open_payload, pair_key/forget_pair_key, sign, next_send_counter, observe_counter → Freshness, next_announce_counter, announce_last, random, store_secret), src/link/tcp_lan.rs (TcpLan::listen / dial; the fabric never binds — you do, per FabricSettings.listen_lan / hub_role), src/store (Store::open/OpenParams, DiskSpace, complete_partials), src/test_double.rs (how the tests fake the Sealer); id111 (2-identity/crates/id111: keyset, seal (caller-owned inner variants from I1), counters (observe_counter A11), store/pins (apply_announce), the announce counter commit); node111 daemon/mod.rs (daemon_main: where the identity handle lives today, the services registry, SSE publish, api_v1 router, proposals.rs KINDS + acceptance, cli.rs), auth::DaemonToken, api_v1/{panels,nearby}.rs as route templates, tests/api_v1_panels.rs as a test template; the contract openapi.yaml (messages*, contacts*, fabric*, fabric/hub/*, fabric/settings, dismissHubOffer = DELETE /fabric/hub/offer; SSE events message/contact/fabric); the A1 proposal body contract (5-messaging/TASKS.md A1 row).
Owned: 1-bootstrap/crates/node111/src/daemon/fabric/** (new), src/daemon/dashboard/api_v1/{messages.rs,contacts.rs,fabric.rs} (new), src/daemon/mod.rs (hooks), src/cli.rs, src/daemon/dashboard/api_v1/proposals.rs (message kind), 1-bootstrap/crates/node111/Cargo.toml + the workspace Cargo.lock (the mesh111 path dependency), tests/api_v1_messages.rs (new), 4-interface/api/openapi.yaml (additive only: Proposal.kind + message, reason_code + not_enrolled + cancelled, Problem.code + restore_pending, identifyDevice description → "USB board (Linux) or meshtasticd; Wi-Fi boards are bound by address in a later task") + the regenerated 4-interface/ui/src/lib/api/{types.gen.ts,sse.gen.ts} and 4-interface/ui/test/contract.gen.ts via pnpm gen:types (nothing else under 4-interface/ui), 5-messaging/README.md ONLY to append an as-built note under §15 if a contract code is added (say so).
Build: (1) daemon/fabric/sealer.rs — Sealer over the live id111 keyset handle: pair_key cache per peer (zeroized on forget_pair_key), seal_payload/open_payload over I1's caller-owned-inner variants, sign with the ASCII domain tag, next_send_counter / observe_counter (Freshness verdict from the A11 window, high-water advanced either way) over id111's CounterStore, next_announce_counter = the committed announce counter (RestoreFloorRequired ⇒ error), announce_last, random from the §4 entropy discipline, store_secret = BLAKE2b-256(key = X25519 private key bytes 32..64, data "111/store-secret/v1") computed behind the handle (the private key never crosses; zeroize); (2) store open at daemon start (Mode S key from the handle, fabric/ dirs under the layout, DiskSpace via statvfs through whatever node111 already depends on — if nothing provides it, report and use the conservative default the store offers), complete_partials delivered at start if the fabric does not do it itself; (3) Fabric::start inside the daemon runtime; tcp-lan listener bound when FabricSettings.listen_lan (LAN address) or hub_role says so, dialer for configured contact addresses, FabricEvent::Settings persisted to fabric/settings (0600); (4) routes §15 exactly as the contract: messages (threads, thread with limit/since_seq, POST send → 202 (RestorePending ⇒ 409 Problem.code restore_pending — additive contract code; record), retry, cancel), contacts (list/add by card|node_id/get/patch/forget, own card, requests accept = add), fabric (status, settings PATCH loopback-only, hub offer POST/DELETE, enroll), all behind the dashboard session or the agent bearer (read-only routes only for the bearer — never settings/offer/enroll/send); (5) SSE: message/contact/fabric events published from the FabricEvent subscription, coalesced per message per tick as the fabric emits them; (6) Facts.fabric provider → the registry rule (fabric service row LIVE/NOT SET/ERROR); (7) proposals.rs: KINDS gains "message"; acceptance of a message proposal parses the A1 body contract ("to: " + 64 lowercase hex + "\\n\\n" + text verbatim), checks the proposal's node-id stamp equals this node, checks the recipient is an accepted contact, then calls the fabric send — the proposal is marked applied only after the store commits the message; (8) CLI: 111 fabric offer|enroll <code>|status over the loopback API (session-less trusted loopback where the dashboard already allows it); (9) shutdown: Fabric::abort + store close (clean flag).
Tests (tests/api_v1_messages.rs with a TestSealer-backed or real-keyset fabric on a temp root): every route's happy path + auth matrix (session / bearer read-only / nothing), SSE coalescing (a 16-fragment transfer ⇒ one event per tick), proposal → send (bad stamp, non-contact, request-pending refused; happy path marks applied after commit), settings/offer loopback-only, restore_pending 409, listener bound only when settings say so, reason_codes not_enrolled and cancelled pass the contract validator. The live two-daemon netns run is V1's (Wave 3) — do not build it; leave a tests/README note on how F2's pieces compose for it.
Check: ${ROOT}/1-bootstrap/scripts/check.sh cargo stages + cd ${ROOT}/4-interface/ui && pnpm run gen:types -- --check && pnpm test (contract tests).` },
]

const RESULT = {
  type: 'object',
  required: ['task', 'status', 'files', 'check', 'deviations', 'notes'],
  properties: {
    task: { type: 'string' },
    status: { type: 'string', enum: ['done', 'partial', 'blocked'] },
    files: { type: 'array', items: { type: 'string' } },
    check: { type: 'string' },
    deviations: { type: 'array', items: { type: 'string' } },
    notes: { type: 'string' },
  },
}

phase('Wave 2')
log('Layer 5 Wave 2 (resumed after the session crash; B1 + U2 landed): F2 alone')
const out = (await parallel(TASKS.map(t => () =>
  agent(CTX + '\n\n' + t.brief, { label: `L5:${t.id}`, phase: 'Wave 2', schema: RESULT, model: t.model, effort: t.effort })
    .then(r => r && { ...r, id: t.id })
))).filter(Boolean)
log('returned: ' + out.map(r => `${r.id}=${r.status}`).join(' · '))
return out
