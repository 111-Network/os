export const meta = {
  name: 'layer-7-wave-0c',
  description: 'Layer 7 Wave 0c: M5b fixes the adversarial-review findings on the daemon fabric seams (M5), then an independent read-only re-review; the lead re-runs the gates and commits',
  phases: [
    { title: 'Fix', detail: 'M5b: the stored announce authoritative for the home-hub rebuild, pair halves published and saved under the lock, settings re-read after a pump lag, the §7.7 trial refusals' },
    { title: 'Re-review', detail: 'one read-only adversarial pass over the M5b diff' },
  ],
}

const ROOT = '/home/user/os'
const SP = '/tmp/claude-0/-home-user-os/e7852691-6eb6-50f1-9e84-b01d744ea3a9/scratchpad'
const a = args || {}
const ONLY = Array.isArray(a.only) ? a.only : null

const CTX = `
You are a BUILD subagent for the 111network OS, Layer 7 (Relay & Presence). Today is 2026-11-02. Repo root: ${ROOT} (a git repo on branch
claude/layer-6-implementation-006mte; you NEVER run git commit/add/stash/checkout/reset: the lead reviews and commits). You will not be messaged
mid-task; everything you need is in this brief and on disk.

THE LAW: the locked spec ${ROOT}/7-relay/README.md (v1.0, D32; §0 gives the path abbreviations M/ N/ NI/ ID/ A/ UI/ API RS/ LC/ L5:), the Layer 5 spec
${ROOT}/5-messaging/README.md (amendments 1-12 are in it, grep "Layer 7 amendment"), and the board ${ROOT}/7-relay/TASKS.md (rows M4 and M4b in the
Wave 0 table: read both whole; the ✅ rows carry as-built facts). Decisions: ${ROOT}/plan/DECISIONS.md D30, D31, D32. Do not re-open decisions. If
the spec is ambiguous, pick the reading that is safest (fail closed, lighter on the Pi floor) and record it under "deviations". Owner rule in
AGENTS.md: the Raspberry Pi is the floor: no resident process, poll, timer, listener or runtime dependency on a non-hub node; on a hub, no work
that grows faster than linearly with the rows held.

HOUSE RULES: Rust edition 2021, typed errors (never panic on input; sizes checked before parse), doc comments citing the spec section ("§x.y"),
unit tests in-module and integration tests under tests/, rustfmt + clippy -D warnings clean (CI runs clippy --all-targets -D warnings on both
workspaces); no new dependency (mesh111's set is final); no wall clock in any ordering or cryptographic decision; foreign bytes are untrusted
everywhere; never a model name or identifier in any file; no attribution lines; never commit; never touch plan/, the boards or the spec.

BOX: 4 CPUs, 15 GB RAM, a fixed disk allowance (about 9 GB free; do NOT cargo clean, do NOT create another target dir). Shared WARM target dirs,
which cargo's own lock serializes: 5-messaging (mesh111) → CARGO_TARGET_DIR=${SP}/target/mesh111 · 1-bootstrap (node111) → ${SP}/target/node111.
Always export the right one with CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=2 and TMPDIR unset before any cargo command, from that workspace's
directory. At most one cargo build of yours at a time. Iterate with narrow commands and run the full check once at the end. Toolchain: cargo 1.97
stable; no musl cross toolchains (musl stages of check.sh SKIP: say so). Scratch files under ${SP}/M5b/ only. Linux hygiene: nothing of yours
stays running when you return (ps and ss check), delete your scratch.

RETURN (structured): task id, status (done | partial | blocked), files changed or created (exact paths), the check command and its tail (at most
25 lines), deviations (each with why), notes for the reviewer (what to read first, open questions, and the exact names other rows must use).
Factual; the lead reads diffs.
`

const FINDINGS = `
THE FINDINGS (an independent adversarial review of M5's diff at commit ca4c514; line numbers are in the files at that commit; re-check each
against the file as it now is — M4b, the mesh111 fix wave, has landed since and touched N/fabric/settings.rs for two new PairedHub bools).
Fix every CONFIRMED one and the PLAUSIBLE ones unless you show in "deviations" why the path is not real.

F1 CONFIRMED (medium): rule 2 of rebuild_home_hub (N/fabric/settings.rs ~671-679, the \`fetched\` set, \`[one] => Some(*one)\`) can rebuild a
  PAIRED hub as the home hub, which §19 C7 forbids: mailbox cursor rows are never deleted (mesh111's advance_cursor only inserts or updates;
  Store::contact_forget deletes only from contacts; Fabric::forget clears home_hub but leaves the cursor), so a former home hub keeps cursor > 0 for
  good. Scenario: A is a member of B (cursor 5); A forgets B, turns hub_role on and pairs with B (whole); A's fabric/settings is deleted and A
  restarts: rule 1 has no hint, rule 2 finds fetched = [B], B becomes A's home hub and A's re-minted announce names B; nothing filters B out (no
  salvage for a deleted file). FIX: the stored announce is authoritative whenever one is valid: IdentitySealer::stored_hub_hint returns three
  states (no valid announce stored / an announce with no hint / an announce with a hint); with an announce and no hint, rebuild nothing (rule 2
  included); with a hint, every rule's pick must match it; fall back to the cursor rule only when no valid announce is stored. Unit-test the
  scenario (a pair with cursor > 0 and an announce without a hint → None).

F2 PLAUSIBLE (medium): the PairTable and the file can be published from a stale snapshot. on_settings (N/fabric/mod.rs ~1001-1021), set_pair_link
  (~1131-1151) and forget_pair_link take their snapshot under persisted.lock() but call publish_pairs and save after the lock is released, so two
  concurrent callers can publish and save in the wrong order (the pump's on_settings for an unpair of C, then a route's set_pair_link(B) publishes
  and saves, then the pump publishes and saves its older snapshot: the wrap's table loses B's pin and the file loses B's half; after a crash B's
  row reads back with nulls). Related: \`gone\` is computed against whatever \`s\` arrives; an older Settings event applied after a newer one drops
  B's fresh half for good. FIX: publish_pairs and the save bytes are produced inside the persisted critical section (a std Mutex: no await inside;
  write the file after, from the bytes, in order — a small sequence number or the lock held through an atomic_write of pre-built bytes); drop a
  half only for a record absent from the fabric's CURRENT settings (re-read with l.fabric.settings() before dropping) or in an explicit unpair path.

F3 PLAUSIBLE (medium): a pump lag silently loses the Settings event, and with it an unpair. On \`Lagged\` the pump (~1610-1640) adds only a
  synthetic Fabric event and never re-reads the settings; the daemon keeps the half and keeps dialling the unpaired hub, the file keeps the
  record. FIX: on a lag, call self.sync_settings() (one actor call) before publishing, and say in the module doc and in your notes that after a
  lag the tap's consumers see one Fabric event meaning "re-read everything" (D11: re-take every carrier pair's owed frames; D2/D3: re-read status).

F4 PLAUSIBLE (low): replace_home_hub_address (~1230) can accept a connection whose handshake named another node: TcpLan::maintain is a no-op when
  the address is already a target (tcp_lan.rs ~794-806) and connected_at accepts any entry in conns at that address, checking up() before waiting
  (an M4 pairing dial with no daemon half, or an in-flight enrolment dial, already at that address). FIX: refuse (Invalid) when Link::peers
  already holds the address before calling maintain, or require the connection to be attributed to the hub (the fabric's status) before Ok(true).

F5 PLAUSIBLE (low): the §7.7 override can mask a later fabric change: apply_rehome's first arm matches (hub, from) alone; if the hub moves back to
  A and the owner forgets and re-enrols at A while the pump never applies the intermediate "no home hub" settings, the fabric's {hub, A} is
  rewritten to the dead B on disk and in GET /fabric. FIX: the smallest sound rule you can show (clear the override whenever a Settings event or
  a Contact event shows the hub forgotten or re-enrolled; or hold a settings generation); state it in the doc comment.

F6 (spec-level, low; for the lead's facts, no code unless trivial): far_holds_address = true with an unreachable far address (A shows a private
  LAN address in its bundle; B on another network records it and dials forever while A waits) gives zero connections. Note in your notes for D6:
  set far_holds_address only when the far hub will record the address (its dial_refusal passes); the lead records the case.

F8 (from the M4b re-review, low): settings::pair_dials dials every recorded tcp pair; with the two persisted bools (M4b: PairedHub.ours/.theirs,
  whole() = both) it should dial only WHOLE pairs: a half pair's second ENROLL rides the fabric's own pairing dial (M4b keeps that dial while the
  pair is half and releases it once whole), so the daemon's §4.6 step 5 dial is for whole pairs only. Also: a settings file written before M4b
  decodes its rows as half pairs (both bools absent → false); say so in the module doc (such rows stop forwarding until re-paired; there is no
  such file outside this branch).

F7 minor: (a) set_pair_link does not refuse an address that a configured peer or the home hub already uses (pair_link_refusal, settings.rs ~305):
  maintain then keeps the other node's expected and the pair can never connect — refuse it; (b) the tap in publish (~1450) runs before the
  batch's Settings is applied (~1470), so a consumer reading pair_table() on that batch's Fabric event sees the old table — apply Settings first
  (or tap after), keeping event order otherwise; (c) relay::tap is a free function with no handle: say in notes that D2, D3 and D11 keep their
  on/off state in process statics (or add the one handle seam the lead can hand to D10 — do not build the consumers).

Also keep: every existing test green (lib fabric, settings_lan, dashboard_auth, api_v1_status_services; api_v1_messages has one root-only
failure on this box, message_proposals_are_checked_then_applied_once, not yours); the Layer 6 file bytes pinned by
no_pair_writes_the_bytes_a_layer6_build_wrote; the dial-once and §7.7 end-to-end tests.
`

const T = {
  M5b: { model: 'opus', effort: 'high', brief: `TASK M5b (row "| M5b |" in the Wave 0 table of ${ROOT}/7-relay/TASKS.md): fix the review findings on M5
(row "| M5 |", landed at ca4c514; read the "Layer 7 seams" section of the module doc of N/fabric/mod.rs first, then settings.rs: PairLink, PairTable,
decode_pairs, encode, rebuild_home_hub, pair_dials; then mod.rs: on_settings, reconcile_pair_dials, set_pair_link, replace_home_hub_address, the
pump and publish). Files you own: ${ROOT}/1-bootstrap/crates/node111/src/daemon/fabric/mod.rs, ${ROOT}/1-bootstrap/crates/node111/src/daemon/fabric/settings.rs,
${ROOT}/1-bootstrap/crates/node111/src/daemon/fabric/sealer.rs. Nothing else (the relay module stubs are other rows'; mesh111 is not yours: if a
mesh111 change would be the better fix, say so in deviations and do the daemon-side fix).
${FINDINGS}
CHECK (all green, paste the tail): from ${ROOT}/1-bootstrap with CARGO_TARGET_DIR=${SP}/target/node111: cargo test -p node111 --lib fabric (three
times); cargo test -p node111 --test settings_lan --test dashboard_auth --test api_v1_status_services; cargo clippy --workspace --all-targets --
-D warnings; cargo fmt --all -- --check.
NOTES must list the exact names that changed (stored_hub_hint's new return type, any new pub fn), the lag rule for the tap's consumers, and the
far_holds_address rule for D6.` },
}

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

const REVIEW = {
  type: 'object',
  required: ['findings', 'sound', 'coverage_gaps'],
  properties: {
    findings: { type: 'array', items: { type: 'object', required: ['severity', 'verdict', 'where', 'defect', 'scenario', 'fix'], properties: {
      severity: { type: 'string', enum: ['high', 'medium', 'low'] },
      verdict: { type: 'string', enum: ['CONFIRMED', 'PLAUSIBLE'] },
      where: { type: 'string' }, defect: { type: 'string' }, scenario: { type: 'string' }, fix: { type: 'string' } } } },
    sound: { type: 'array', items: { type: 'string' } },
    coverage_gaps: { type: 'array', items: { type: 'string' } },
  },
}

const run = (ids, ph) => parallel(ids.filter(id => !ONLY || ONLY.includes(id)).map(id => () =>
  agent(CTX + '\n\n' + T[id].brief, { label: `L7:${id}`, phase: ph, schema: RESULT, model: T[id].model, effort: T[id].effort })
    .then(r => r && { ...r, id })
)).then(rs => rs.filter(Boolean))

phase('Fix')
const F = await run(['M5b'], 'Fix')
log('fix: ' + F.map(r => `${r.id}=${r.status}`).join(' · '))
if (a.stopAfter === 'fix' || !F.length || F[0].status === 'blocked') return { F }

phase('Re-review')
const R = await agent(`You are a read-only adversarial reviewer of one Rust change in ${ROOT} (the 111network mesh OS). Do NOT run cargo, do NOT edit any
file, do NOT start any process; read only. The change: the uncommitted working-tree diff of 1-bootstrap/crates/node111/src/daemon/fabric/{mod,settings,sealer}.rs
(run: cd ${ROOT} && git diff --histogram -- <those paths>). It is task M5b, the fix of these findings on the earlier change (read them first, they are
the claims to verify):
${FINDINGS}
The author's own report: ${JSON.stringify(F[0]).slice(0, 6000)}
Spec: ${ROOT}/7-relay/README.md §4.6, §7.7, §19 (C7). Verify: (1) each finding is actually closed by a real code path (trace it, cite file:line in
the CURRENT file); (2) no Layer 6 behaviour changed beyond the declared ones (the file bytes without a pair, a member's home-hub rebuild with a
valid announce); (3) no await inside a std Mutex critical section, no new task, timer, poll or socket; (4) the tests exercise the claims. Report
only what you can show with a concrete path; CONFIRMED when traced end to end, PLAUSIBLE otherwise; list what you checked and found sound and
what is not covered.`,
  { label: 'L7:M5b-review', phase: 'Re-review', schema: REVIEW, model: 'opus', effort: 'high' })
log('re-review: ' + (R ? `${R.findings.length} findings` : 'none'))
return { F, R }
