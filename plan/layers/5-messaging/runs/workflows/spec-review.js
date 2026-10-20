export const meta = {
  name: 'layer-5-review',
  description: 'Layer 5 spec adversarial panel: 3 fable judges (crypto/envelope, panels+Meshtastic security, integration/scope)',
  phases: [{ title: 'Judge', detail: '3 fable judges in parallel, each writes a review file and returns structured findings' }],
}

const ROOT = '/home/kia/projects/111network/os'
const OUT = ROOT + '/plan/layers/5-messaging/review'

const CTX = `
You are an ADVERSARIAL JUDGE on the Layer 5 (Messaging & Data) spec of the 111network OS. Today is 2026-10-14.
Repo root: ${ROOT}. Your job is to ATTACK the spec before it locks: find what is wrong, unbuildable, dishonest,
insecure, inconsistent with the locked sibling layers, or reinvents something an adopted project already provides.
Default to skepticism; a finding you cannot ground in a quoted line of the spec or a sibling document is not a finding.

READ FIRST (in this order; use sed -n / grep, do not slurp unrelated trees):
1. ${ROOT}/5-messaging/README.md — the spec under review (whole file, ~450 lines).
2. ${ROOT}/plan/DECISIONS.md — D13 (identity wire contract), D17 (device decisions), D18 and D19 (this cycle's owner
   decisions: the spec MUST implement them, a judge does not relitigate owner decisions but may show they are
   mis-implemented or have unstated consequences).
3. ${ROOT}/plan/layers/5-messaging/options.md — the options brief the spec came from (§0 facts, §3 detail picks, §4 amendments).
4. ${ROOT}/plan/layers/5-messaging/research/00-code-seams.md — where the spec plugs into built code (line-verified map).
5. Lens-specific reading listed in your brief below. Research reports in ${ROOT}/plan/layers/5-messaging/research/ (01–07) hold the
   evidence with URLs; cite them when a spec claim contradicts them.

HARD RULES the spec must respect (from the owner, not negotiable): adopt, don't reinvent · Reticulum is a connected
service, no RNS import anywhere (${ROOT}/LICENSING.md lines 20–35) · the L2 sealed-packet construction and announces are
fixed (amendments may be proposed, listed in spec §19) · no wall clock in crypto or ordering · no runtime downloads · no
telemetry · no policy limits (no rate caps, no expiry, no per-sender quotas; capacity and physics only) · GPL code never
links into our Rust binary (separate processes/bundles are fine) · Linux musl-static + Android/Termux (no root) targets.

METHOD: you may run cheap real checks (python arithmetic on byte budgets, parse the TOML manifest snippet with
python3 -c "import tomllib" (Python 3.11+) or tomli if present, grep the Rust code named in 00-code-seams.md, read
${ROOT}/2-identity/crates/id111/src/{seal.rs,counters.rs,announce.rs,store.rs} headers). Do NOT build, do NOT modify any
repo file except your own review file. Cost discipline: ≤ 25 tool calls on reading; spend your effort thinking.

OUTPUT: write your review to the path given in your brief (markdown, ≤ 2500 words): 1. Verdict (lock-ready |
fix-then-lock | not-ready, with a 5-line justification) · 2. Findings table sorted by severity (id · severity
blocking/major/minor · spec section · title) · 3. Each finding: the quoted spec line(s), the attack or defect in
concrete terms (inputs → wrong outcome), the fix (specific text or design change), and whether you verified it (a
real check) or reasoned it · 4. What is right (≤ 5 bullets; the lead needs to know what NOT to touch). Then return the
structured verdict. Severity: blocking = the spec cannot lock with this (security hole, unbuildable, contradicts an
owner decision or a locked sibling contract); major = must be fixed before build but the architecture stands; minor =
wording, tightening, nice-to-have.
`

const JUDGES = [
  {
    key: 'crypto',
    file: 'crypto.md',
    brief: `LENS: CRYPTO / ENVELOPE / PROTOCOL (spec §3–§12, §18, §19 "Layer 2 amendments").
Extra reading: ${ROOT}/2-identity/README.md §6 (all), §8.4, §9, §10, §12, §13, §15; ${ROOT}/plan/layers/5-messaging/research/04-envelope.md
and 07-gap-2.md; id111 seal.rs (header, inner pin, MIN_PACKET_LEN), counters.rs (window, batch commit, restore jump).
Attack list (not exhaustive): (1) pair_mac: is the KDF/MAC construction sound (keyed BLAKE2b, key derivation inputs,
what it covers, ordering of verification vs window update), sender_ref 4 B collisions and the "try each candidate" rule
(DoS, confusion, cross-peer window poisoning), the claim that pair_mac fixes Noise-"N" for routine traffic — does it,
and is deniability actually what is wanted for which kinds? (2) Fragment-then-seal: outstanding ≤ 16 vs the 32-entry
window, re-seal on retransmit, reassembly state bounds, fragment-index abuse, memory exhaustion on hubs. (3) Tier-2
unsealed chunk carve-out: what does an on-path observer or a malicious hub learn or alter; session_tag linkability;
is "already encrypted to its recipient" well-defined for attachments (object_key inside the envelope). (4) Message
identity (sender, stream, msg_seq): idempotency under restore (2^32 jump), under the sender reusing seq after a crash
before the store commits (is msg_seq persisted before send?), ACK semantics (cumulative prefix + ranges) under
reordering and partial fragments, reflection/replay of ACKs, duplicate suppression at hubs by epk[0..8]. (5) Custody:
CUSTODY_ACK binding (BLAKE2b-128 over exact bytes), enrollment code (keyed hash, constant-time, replay, offline
brute-force of 8 chars), what a malicious hub can do (drop, reorder, replay items, learn metadata), MAILBOX_COMMIT
semantics. (6) Contacts: INTRO/TOFU message requests (key-substitution attacks, announce-inside-ciphertext trust,
card request answered from a hub cache: can a hub serve a stale or wrong announce, and does the "verify against the
expected NodeId" rule close it), immutability vs legitimate re-key. (7) Store key HKDF from the keyset: domain
separation, what else is derived from the keyset, salt choice. (8) tcp-lan with no transport crypto: metadata leak
statement complete? hello/announce frame spoofing, keepalive abuse, frame length bounds. (9) Groups pairwise fan-out
and reserved flags: do reserved-bit rejections and the GROUP/RATCHET bits create downgrade or future-compat traps?
(10) Domain tags and the L2 amendment list A1–A10: is each necessary, sufficient, and append-only-safe? Anything the
spec silently changes in L2 without listing it? (11) Any place a wall clock sneaks in. (12) Cat-1 relay parsing: can a
stock MeshCore repeater actually carry these frames (163 B) with the L6 shim as written?
Write to ${OUT}/crypto.md`,
  },
  {
    key: 'panels',
    file: 'panels.md',
    brief: `LENS: HOSTED-PANEL PROXY SECURITY + MESHTASTIC INTEGRATION (spec §13, §14, §15 panel/identify routes, §18 items 7–8).
Extra reading: ${ROOT}/4-interface/README.md §11 (auth as built: pair code, s111 cookie, loopback trust, CSRF rules, host
guard), §6.6, §7; ${ROOT}/plan/layers/5-messaging/research/03-hosted-panels.md (mechanics + sources) and 07-gap-3-meshmonitor.md;
${ROOT}/1-bootstrap/crates/node111/src/daemon/dashboard/routes.rs (CSP/headers middleware, ~line 70) and
api_v1/auth.rs (cookie/session/CSRF code) — read enough to judge the interplay; D16(a) loopback policy in DECISIONS.md.
Attack list: (1) Ticket flow: single-use 128-bit ticket in a URL (referrer leak, history, logs), 30 s window, who may
mint (dashboard session vs trusted loopback), ticket → cookie swap on the panel origin, cookie attributes
(HttpOnly, SameSite=Strict, no Domain, Path), cookie-jar sharing across ports on one host (RFC 6265 §8.5): can a
malicious panel read/set/shadow s111 or p111_* of another panel; does stripping cookies upstream fully protect the
dashboard; what about localStorage/origin isolation. (2) CSRF guards: Sec-Fetch-Site support on the browser floor
(Chrome 111+/Safari 16.4+/Firefox 128+ and Android WebView) — is the Origin==Host fallback safe; GET-based state
changes in adopted apps (Meshtastic web client HTTP mode uses PUT /toradio and GET /fromradio — is GET harmless?).
(3) Header rewrite: X-Frame-Options removal + frame-ancestors replacement, CSP parsing edge cases (multiple headers,
report-only), Set-Cookie Domain stripping, hop-by-hop headers, WebSocket upgrade through the proxy, max_body, slowloris
on panel listeners, Host guard list correctness when the user types an IP vs 111.local. (4) iframe sandbox +
allow: does sandbox="allow-scripts allow-same-origin" defeat the purpose; permission delegation for serial/bluetooth on
a cross-origin frame; frame-src list maintenance; clickjacking of OUR dashboard by the panel (frame-ancestors 'none'
on dashboard — ok?) and of the PANEL by third parties. (5) Secure-context gate honesty: Meshtastic needs_secure_context
= false while allow includes serial/bluetooth — is the copy honest on LAN and on loopback (http://127.0.0.1 vs
http://111.localhost vs http://localhost on Android Chrome)? (6) LAN exposure: panel listeners bound on LAN when LAN
sharing is on — does the unpaired-visitor story hold (ticket needs a session), and what does an unpaired LAN visitor
see on a panel port? (7) Offline lint and the GPL static bundle: build-time verification, source offer, no runtime
fetch — any CDN/fonts/map tiles in the Meshtastic web client? (8) Meshtastic: meshmud holding the one API slot, one
HTTP client, identify route consent + port lock + 409 busy, presence file written by a GPL Python child and parsed by
the daemon (size bounds, schema validation, path/symlink safety, atomic write), "read-only" definition enforced how
(the panel CAN transmit through meshmud — is the service copy honest that the user's panel actions transmit?),
meshmud config rendering from interfaces.toml (injection), vendoring/pinning claims vs no-runtime-download rule,
Termux viability of a Go binary + Python venv. (9) Contract growth: are the new routes/enums/fields additive-only and
consistent with openapi conventions (problem+json, 409 codes)? (10) Anything in §13 that D15/D16 already decided
differently and the spec silently flips.
Write to ${OUT}/panels.md`,
  },
  {
    key: 'integration',
    file: 'integration.md',
    brief: `LENS: INTEGRATION, SCOPE HONESTY, BUILDABILITY, ADOPT-DON'T-REINVENT (whole spec; emphasis §1–§2, §8–§11,
§14–§17, §19–§22).
Extra reading: ${ROOT}/README.md §5 and "What We Build vs. What We Adopt"; ${ROOT}/4-interface/README.md §5–§7, §10;
${ROOT}/4-interface/api/openapi.yaml (ServiceId, ServiceRecord, DeviceRecord, x-sse-events); ${ROOT}/3-intelligence/README.md
(tools, contracts); ${ROOT}/1-bootstrap/README.md (state layout, transport, update path) and 1-bootstrap/TASKS.md
"Dependency policy"; ${ROOT}/plan/cross-layer/devices.md §2–§3, §7; ${ROOT}/plan/cross-layer/parked.md; research 01, 02, 05, 07-gap-1, 07-gap-2.
Attack list: (1) Does the spec implement D18 and D19 exactly (every decision, including the amendments D19 c made to
cross-layer/devices.md and D19 d made to D15), with no silent additions or omissions? (2) Honesty of the four delivery states and of
"Messaging LIVE": can a user be shown something the network did not confirm? (3) Buildability in one cycle: list every
custom piece and judge whether it is small glue or a hidden subsystem (e.g. the proxy, the mailbox, Tier-2 bulk, the
identify route, the presence reader, contacts/QR, the Messages UI); which should be cut to a seam for v1 and which are
under-specified to the point a builder would guess. (4) Adopt-don't-reinvent: for each custom piece, name the adopted
candidate that was considered (from the research) and whether the spec's reason for not using it holds. (5) Seams: are
ports, state dirs, ServiceIds, SSE event names, agent tools, CLI verbs, manifest entries and child-process patterns
consistent with the built code in 00-code-seams.md and the Layer 1/3/4 specs (e.g. ChildSpec needs a binary in
current/bin — where do meshmud and the presence venv live; who chowns fabric/; how does 111-agent reach /api/v1 given
its loopback allowlist; does the services registry's fixed-size-7 arrays get the two new ids; does the radio service
keep a coherent meaning next to meshtastic)? (6) Layer 6 "with L6 in mind": does the Link trait + 160 B profile +
interfaces.toml owner rule really let Layer 6 add kiss-modem without touching the envelope or store? (7) Phones: what
in v1 actually works on a Termux node (hub enrollment over TCP, store on Android, panel on the phone's own browser with
loopback not trusted under Termux — D16 a)? (8) KB rubric: are the §22 entries real projects with verifiable licenses,
and is anything adopted without an entry? (9) Verification gates §20: are they runnable in a netns with the existing
smoke harness; what is untestable and therefore an honest external gate? (10) The deferred list §21: anything deferred
that the README §5 promises for the layer itself ("the same fabric carries… updates and validated patches") and the
spec should at least seam? (11) Licensing: GPL source offer for firmware/bundles in the manifest, meshmud (MIT) using
GPL protobufs, the presence reader's venv — anything that contradicts LICENSING.md or needs a line there.
Write to ${OUT}/integration.md`,
  },
]

const REVIEW_SCHEMA = {
  type: 'object',
  required: ['lens', 'verdict', 'summary', 'findings', 'right'],
  properties: {
    lens: { type: 'string' },
    verdict: { type: 'string', enum: ['lock-ready', 'fix-then-lock', 'not-ready'] },
    summary: { type: 'string', description: '≤ 120 words' },
    findings: {
      type: 'array',
      items: {
        type: 'object',
        required: ['id', 'severity', 'section', 'title', 'problem', 'fix', 'verified'],
        properties: {
          id: { type: 'string' },
          severity: { type: 'string', enum: ['blocking', 'major', 'minor'] },
          section: { type: 'string' },
          title: { type: 'string' },
          problem: { type: 'string', description: 'concrete: inputs → wrong outcome, ≤ 80 words' },
          fix: { type: 'string', description: 'specific change, ≤ 80 words' },
          verified: { type: 'boolean', description: 'true only if a real check (arithmetic, code read, parse) was run' },
        },
      },
    },
    right: { type: 'array', items: { type: 'string' }, description: 'what is correct and must not be touched (≤ 5)' },
  },
}

phase('Judge')
log('Layer 5 adversarial panel: 3 fable judges')
const results = (await parallel(JUDGES.map(j => () =>
  agent(CTX + '\n\n' + j.brief, { label: `judge:${j.key}`, phase: 'Judge', schema: REVIEW_SCHEMA, model: 'fable', effort: 'xhigh' })
    .then(r => r && { ...r, key: j.key })
))).filter(Boolean)
log(`${results.length}/3 judges returned: ` + results.map(r => `${r.key}=${r.verdict} (${r.findings.filter(f => f.severity === 'blocking').length}B/${r.findings.filter(f => f.severity === 'major').length}M/${r.findings.filter(f => f.severity === 'minor').length}m)`).join(' · '))
return results.map(r => ({
  key: r.key, verdict: r.verdict, summary: r.summary, right: r.right,
  findings: r.findings.map(f => `[${f.id}] ${f.severity.toUpperCase()} §${f.section} ${f.title} — ${f.problem} ⇒ FIX: ${f.fix}${f.verified ? ' (verified)' : ''}`),
}))