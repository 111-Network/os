export const meta = {
  name: 'layer-5-wave-3',
  description: 'Layer 5 build Wave 3: V1 three-node netns smoke (G1) and V2 panel offline lint (G3) in parallel; the lead runs V3 (gates) and then V4 (fable integrated review)',
  phases: [{ title: 'Wave 3', detail: 'V2 panel lint (sonnet) · V1 three-node smoke (opus, last: bounce-prone)' }],
}

const ROOT = '/home/kia/projects/111network/os'

const CTX = `
You are a BUILD subagent for the 111network OS, Layer 5 (Messaging & Data), verification Wave 3. Today is 2026-10-14.
Repo root: ${ROOT} (git; you NEVER run git commit/add/stash/checkout — the lead reviews and commits). You will not be
messaged mid-task; everything you need is in this brief and on disk.

THE LAW: the locked spec ${ROOT}/messaging/README.md (v1.2 + as-built amendments; §20 lists the gates G1–G4) and your row
in ${ROOT}/messaging/TASKS.md. Everything of Waves 0–2 is committed on main: the mesh111 crate (messaging/crates/mesh111),
node111 with the fabric wired in (daemon/fabric/**, routes api_v1/{messages,contacts,fabric,panels,nearby,devices}.rs,
CLI \`111 fabric …\`), hosted panels (daemon/panels/**), Meshtastic (daemon/meshtastic/** with the stub-able meshmud TCP
downlink on 127.0.0.1:11230), agent111 with the Layer 5 tools, the UI with Messages/Contacts/Nearby/Panel tab/Settings,
and the build gates (bootstrap/scripts/check.sh ALL GREEN). Read ${ROOT}/plan/layer-5-review/wave-1.md and the Wave 2
section of ${ROOT}/plan/layer-5-research.md for the readings the builders recorded. The Layer 4 smoke harness
${ROOT}/interface/verify/smoke.sh (unshare -rnpf user+net+pid namespaces, stub-llama.py model, SSE tapes to
interface/ui/test/tapes/) is the template: extend it, do not fork it.

THIS BOX: user namespaces work (\`unshare -rn\` + veth pairs inside the userns, \`ip\` at /usr/sbin/ip); Google Chrome at
/usr/bin/google-chrome (use --headless=new --remote-debugging-port, drive CDP over Node's built-in WebSocket — no npm
deps; playwright's chromium under ~/.cache/ms-playwright may also be used read-only); no Go (meshmud is CI-built, so the
Meshtastic downlink is a STUB TCP server speaking the 0x94 0xC3 len framing — tests/api_v1_meshtastic.rs has one to copy);
Python 3.10; node/pnpm; export PATH="$HOME/.cargo/bin:$PATH". Debug builds of \`111\` and \`111-agent\` are what the smoke
runs (rust-embed debug reads ui/dist from disk). Nothing downloads at runtime. Never commit.

FILE OWNERSHIP IS STRICT: only your row's files (plus new files inside those dirs). One other task runs in parallel.
A change you need elsewhere (a daemon flag, a route): do NOT make it — report "blocked-by: <what>" with the exact gap
and continue with everything else; the lead bounces it.

Every check prints \`ok:\` / \`FAIL:\` / \`note:\` lines like smoke.sh and exits 1 on any FAIL. Every process you start must
die with the namespace (pid ns) — no leftovers. SSE tapes are byte-for-byte daemon recordings (never hand-written); name
them as interface/ui/test/tapes/README says.

RETURN (structured): task id, status (done | partial | blocked), files, the check command + tail (≤ 30 lines), gaps
(each: what failed or could not be asserted, where, suspected owner — these feed the fix wave), deviations, notes.
`

const TASKS = [
  { id: 'V2', model: 'sonnet', effort: 'high', brief: `TASK V2 — panel offline lint + G3 (spec §13 "lint", §20 G3). Build interface/verify/panel-lint.sh + interface/verify/cdp/** (Node, no deps): inside the same kind of namespace smoke.sh uses, start a scratch 111 root with the Meshtastic panel ENABLED against a stub meshmud TCP downlink on 127.0.0.1:11230 (a small Python or Node stub that speaks the 0x94 0xC3 len framing and answers want_config with my_node_info + a few node-info frames; the panel bundle is whatever current/panels/meshtastic/ holds on this box — if the real Meshtastic web client bundle is not present, build the lint so it runs against a MINIMAL placeholder bundle you write under the scratch root that exercises the same API shape (index.html + a script calling PUT /api/v1/toradio and GET /api/v1/fromradio?all=true), and say so under gaps: the real-bundle run is the external gate). Drive headless Chrome over CDP: open the dashboard, pair (smoke.sh shows how), open the service page's Panel tab, follow the ticket URL into the panel origin, let it run 10 s; record EVERY network request (Network.requestWillBeSent) and assert the §20 G3 list: no request leaves the panel origin except to its own origin (no fonts/CDN/telemetry/analytics; 127.0.0.1 only), no WebSocket, no request to the dashboard origin from the panel, CORS headers absent on every panel response, frame-ancestors on the panel response is the dashboard origin, the ticket appears in exactly one URL (the swap) and never again, the p111 cookie is HttpOnly + SameSite=Strict + host-only, the iframe sandbox/allow attributes match the manifest, a cross-origin fetch of the panel's /api/v1/fromradio from the dashboard origin is refused (403) with no CORS headers, and the dashboard's CSP frame-src lists only the enabled panel origin. Also lint the static bundle itself offline: every script/link/img src in the served HTML is same-origin and relative; no inline event handlers; report the bundle's third-party hostnames found by grep (information). Print a table of requests (origin, path, type) at the end.
Owned: interface/verify/panel-lint.sh, interface/verify/cdp/** (new), interface/verify/stub-meshmud.py (new) if you write the stub in Python, interface/ui/test/tapes/README.md (a line if you add a tape).
Check: bash interface/verify/panel-lint.sh green (exit 0) on this box; paste the request table.` },
  { id: 'V1', model: 'opus', effort: 'xhigh', brief: `TASK V1 — three-node smoke (spec §20 G1; §9 scenarios). Extend interface/verify/smoke.sh with a THREE-NODE MODE (flag --three-node; the Layer 4 single-node mode must keep passing unchanged): one user+pid namespace, three net namespaces (hub H, members A and B) joined by veth pairs to a bridge (or a hub-and-spoke of veths; 10.111.0.0/24), three scratch roots with their own identities (111 identity generate), real 111d + 111-agent + stub-llama in each, LAN sharing on where the scenario needs it, the hub role enabled on H via its settings/CLI (111 fabric offer / enroll <code>, or the loopback API — read daemon/fabric/** and cli.rs for what landed). Scenarios, each with assertions through the public API (curl + python3 -I json) and SSE tapes recorded from each node:
(1) cards: A and B add each other by pasted card (GET own card on one, POST contacts on the other) — contact has_key true both ways;
(2) direct: A ↔ B on the LAN: A sends, B's thread shows it, A's message reaches delivered; a 16-fragment-sized message (force the tcp-lan profile; if a size knob is not exposed, send > 64 KiB and assert the fragment count in Advanced) delivers once; SSE on A shows one coalesced event per tick for it;
(3) hub: H offers, A and B enrol with the code (one code each); A goes down (kill its netns link: ip link set down), B sends to A → relayed with H's custody; A comes back → delivered exactly once; B's SSE shows sent → relayed → delivered;
(4) non-member: a third identity (not enrolled) is the dest of B's message → sent + reason hub_not_yours (REJECT_UNKNOWN_DEST), shown honestly;
(5) replay: capture A→B sealed TCP frames with a tiny Python proxy or tcpdump-less socket relay (a pass-through TCP relay you write that records frames) and replay them to B on a fresh socket: B never attributes that connection (no push, no card answer), no duplicate delivery (B's thread unchanged), and B's counters show the replay (fabric status);
(6) restore: restart A's daemon with an unclean shutdown (kill -9) → A's next messages use a new epoch (Advanced shows epoch jump); nothing is redelivered;
(7) requests: a fourth identity C with no contact entry on A sends to A over tcp-lan (INTRO) → A lists a message request; accept → the held message appears once and is ACKed (C's copy reaches delivered); ignore is client-side (nothing to assert on the node beyond "still pending");
(8) agent: on A, with the daemon-minted bearer, 111-agent's messages.read / contacts.read / fabric.read / nearby.read answer through /v1/chat tool calls (stub model emits the tool call), messages.send writes a proposal of kind message; accepting it in the API sends the message (B receives it);
(9) Meshtastic: on H with the stub meshmud downlink (tests/api_v1_meshtastic.rs has a stub to copy) and a fake board binding in devices.json, the meshtastic service row is LIVE, GET /nearby lists the stub's nodes with names flagged untrusted, the panel ticket mints (V2 does the browser part);
(10) shutdown: every node restarts cleanly (exit 0, children joined, clean flag set) and the stores reopen without an epoch jump.
Record what could not be asserted as gaps with the exact missing hook (e.g. "no API to force the 160 B profile"). Keep the run under ~10 minutes on this box; use the debug binaries already built by the single-node mode.
Owned: interface/verify/smoke.sh (three-node mode + helpers), interface/verify/**/*.py new helpers (relay, stub meshmud if V2's is not yet there — coordinate by file name: interface/verify/stub-meshmud.py belongs to V2; if absent when you need it, write interface/verify/stub-meshmud-v1.py and say so), interface/ui/test/tapes/** (new tapes + README lines).
Check: bash interface/verify/smoke.sh (single-node, must still pass) && bash interface/verify/smoke.sh --three-node (green, exit 0); paste the ok:/FAIL: summary counts and every FAIL/note line.` },
]

const RESULT = {
  type: 'object',
  required: ['task', 'status', 'files', 'check', 'gaps', 'deviations', 'notes'],
  properties: {
    task: { type: 'string' },
    status: { type: 'string', enum: ['done', 'partial', 'blocked'] },
    files: { type: 'array', items: { type: 'string' } },
    check: { type: 'string' },
    gaps: { type: 'array', items: { type: 'string' } },
    deviations: { type: 'array', items: { type: 'string' } },
    notes: { type: 'string' },
  },
}

phase('Wave 3')
log('Layer 5 Wave 3: V2 panel lint · V1 three-node smoke in parallel')
const out = (await parallel(TASKS.map(t => () =>
  agent(CTX + '\n\n' + t.brief, { label: `L5:${t.id}`, phase: 'Wave 3', schema: RESULT, model: t.model, effort: t.effort })
    .then(r => r && { ...r, id: t.id })
))).filter(Boolean)
log('returned: ' + out.map(r => `${r.id}=${r.status} (${r.gaps.length} gaps)`).join(' · '))
return out
