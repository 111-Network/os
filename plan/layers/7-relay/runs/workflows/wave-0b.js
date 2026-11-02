export const meta = {
  name: 'layer-7-wave-0b',
  description: 'Layer 7 Wave 0b: M4b fixes the adversarial-review findings on the mesh111 forwarder and pairing (M4), then an independent read-only re-review; the lead re-runs the gates and commits',
  phases: [
    { title: 'Fix', detail: 'M4b: pairing wholeness persisted, O(N) forwarder with a resident window, carrier dial release, re-post bound, hub_role gate, stale acks' },
    { title: 'Re-review', detail: 'one read-only adversarial pass over the M4b diff' },
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
stable; no musl cross toolchains (musl stages of check.sh SKIP: say so). Scratch files under ${SP}/M4b/ only. Linux hygiene: nothing of yours
stays running when you return (ps and ss check), delete your scratch.

RETURN (structured): task id, status (done | partial | blocked), files changed or created (exact paths), the check command and its tail (at most
25 lines), deviations (each with why), notes for the reviewer (what to read first, open questions, and the exact names other rows must use).
Factual; the lead reads diffs.
`

const FINDINGS = `
THE FINDINGS (an independent adversarial review of M4's diff at commit 841d90a; line numbers are in M/fabric.rs at that commit; re-check each
against the file as it now is). Fix every CONFIRMED one; the PLAUSIBLE ones too unless you show in "deviations" why the path is not real.

F1 CONFIRMED (high): a member enrolment fools the "whole pair" test. eligible_pairs (~4759) counts a pair as whole when the far hub's contact is
  is_enrolled; Core::pair (~4765-4803) refuses only this node's own home hub; the pairing ENROLL_OK pins the far hub at Oob (~3916-3923), which never
  downgrades an existing Enrolled row; and the hub side of on_enroll pins ANY redeemer of ANY offer at Trust::Enrolled (a member). Scenario: hub B is
  (or once was) A's member (B's home hub is A; L5 has no un-enrol at A). A pairs with B: A's ENROLL rides B's attributed connection, A records B, and A
  now treats B as a WHOLE pair although B never enrolled by pairing (B's own pair() refuses A, its home hub). At A: B stops being a member (its
  MAILBOX_FETCH gets only the item-less answer, its fwd = 0 rows are stranded, its commits are ignored); new puts for dest = B become forward rows that
  B answers REJECT_UNKNOWN_DEST (dest == own) and are then refused; a tcp peer's put becomes a forward row against §5.1's last row.
  THE DESIGN (the lead's reading of §4.6 steps 1-4, binding): wholeness is recorded, never inferred from Trust.
  - A pairing offer is its own call: Fabric::offer_pair(address: Option<SocketAddr>, role: PairRole, via: PairVia) -> Result<OfferView> (hub role
    only; one open offer at a time as today; the bundle fields role/via are the daemon's, D6). The code it mints is a PAIRING code: when the far hub's
    ENROLL redeems it, this hub records (or updates) PairedHub { node_id: far, role, via } with theirs = true, pins the far hub's card at Oob (never
    Enrolled: a paired hub is never a member), answers ENROLL_OK, and persists through FabricEvent::Settings. A member offer's code (offer()) never
    records a pair; an ENROLL from a recorded pair that redeems a MEMBER offer is refused (EnrollStatus::Refused); an ENROLL that redeems a pairing
    offer from this node's own home hub is refused (amendment 12, hub side).
  - This hub's own ENROLL_OK of a pairing (Core::pair → on_enroll_ok) records or updates the same record with ours = true (the role/via the caller
    gave; on a mismatch with an existing record's role/via, Invalid, nothing changed).
  - PairedHub grows two persisted bools: ours (this hub's ENROLL got ENROLL_OK there) and theirs (the far hub redeemed this hub's pairing offer).
    whole = ours && theirs. The crate-root re-export keeps the name; PairedHub::valid() unchanged; normalize_pairs keeps both bools. This is an
    additive deviation from amendment 10's {node_id, role, via} shape: record it; the daemon persists the two bools in its paired_hubs row
    (N/fabric/settings.rs: M5 landed it; you own that file for exactly this: the two keys, default false when absent, and the decode/encode tests).
  - Every forward path admits a pair ONLY when whole: the gate's sender check (custody_decide: a sender is admitted when it is a member (is_member)
    OR a whole, unbroken pair; a half pair's put meets REJECT_UNENROLLED), on_custody_ack's pair arm, on_fetch's pair answer, carrier_inbound,
    forward_take, eligible_pairs, the status view (PairView.whole = the two bools). unpair drops the record (both bools) and everything held for the
    pair; it no longer rewrites the far hub's contact (a member stays a member; a pair's card was never Enrolled).
  - is_member stays is_enrolled && find_pair.is_none() (a hub that is both a member and a pair is a pair; say so in the module doc).

F2 CONFIRMED (high, Pi floor): the forwarder is O(rows) per trigger, so a drain of N rows is O(N²), on the single actor. forward_pass recomputes
  forward_targets for every Idle row on every run (allocations plus a linear scan of the 256-entry cache); triggers include every pair CUSTODY_ACK,
  every contact event (announce refreshes included), attribution, pair fetches, carrier frames and every forward_take. Scenario: 20k rows wait for a
  pair that returns after an hour; each ack runs a 20k-row pass (about 4×10^8 target computations); a Pi stalls for minutes. untrack's
  pending.retain is O(pending) per commit (pending holds every Posted carrier row), so carrier drains are O(N²) too. And load_forward_rows keeps
  every protected row resident: at the 128 MiB Transit default and ~250 B per row that is ~500k rows, ~75 MB RSS.
  THE DESIGN: (a) per-target queues of Idle seqs (custody order) and an in-flight count per target maintained incrementally; targets are recomputed
  only for rows whose inputs changed (a new or reloaded row, a refusal, an orphaning, the pair set or the carrier bound changed, a cache hit for its
  dest) — never for every row on every trigger; the put stage visits only targets that are up and have window room; (b) FwdRow keeps its pkt_ref so
  untrack and forward_done are O(1) (pending becomes a plain map lookup both ways); (c) a RESIDENT WINDOW: at most FORWARD_RESIDENT = 4_096 forward
  rows live in memory; load_forward_rows and the forwarder page the rest from the store in custody order with forward_pending(after, limit) as the
  window drains (M1 built that API for this); FabricStatus/PairView.pending counts the resident rows (document that rows beyond the window are "in
  the store, not yet offered"; forward_stats() has the totals). Add a test-only counter of target computations and assert it stays linear in a
  1,000-row drain (≤ 3 × rows across the whole drain); add a test that a 6,000-row backlog drains completely through the window.

F3 CONFIRMED (medium, Pi floor and §4.6 step 3): a carrier pairing keeps a resident tcp dial. record_pair keeps the attempt's dial whatever via is,
  and dial_held pins it: the dialer redials a carrier pair with back-off until unpair, while the spec says a carrier pair talks only through its
  carrier after enrolment. (The tests hide it: the hand-off test and leaf_to_leaf release the dial by hand; remove those by-hand releases.)
  FIX: keep a pairing dial only while the pair is not whole (so the far hub's ENROLL can ride it); once the pair is whole, release the fabric's
  pairing dial for EVERY via (M5 landed the daemon's §4.6 step 5 rule, "one connection per pair": the daemon's reconcile_pair_dials in
  N/fabric/mod.rs holds the dial for a tcp pair on the side whose NodeId sorts lower, and releases it through Dialer::release; a fabric dial kept
  alongside makes both hubs dial, M5's open question 1); a carrier pair is never dialed by the fabric afterwards. Note for the lead which call the
  daemon uses to learn the pair became whole (a Settings event carrying the two bools suffices).

F4 CONFIRMED (medium): re-post rule (1) has no bound. carrier_inbound re-arms every Posted{repost:false} row on every verified far frame, frame by
  frame; after a re-take + forward_done(Taken) the row is Posted{repost:false} again, so the next far frame re-arms it again; within one drain
  [ack R1, ack R2, …] ack R1 re-arms R2..Rn before their own acks are processed and a Forward event can make the daemon re-take them in between.
  FIX: read §5.9 rule (1) and C6 and implement exactly their bound (at most one rule-(1) re-post per row; a per-row bit, never reset by a re-take);
  in carrier_inbound only set a per-pair "far frame heard" flag and apply the re-arm in the next forward_take, after the whole drain was handed in.
  Test it: a drain of three acks re-arms nothing that those acks commit, and a post is re-armed by rule (1) once, never twice.

F5 CONFIRMED (low): hub_role off leaves pairs live on a custody_opt_in node (holds_custody = hub_role || custody_opt_in; patch_settings flips
  hub_role without touching paired_hubs; normalize_pairs ignores hub_role). FIX: every pairing and forwarding path (pair_of, eligible_pairs,
  carrier_pair, carrier_inbound, forward_take, forward_repost, pair, offer_pair, load_forward_rows, the pair fetch answer) is gated on
  settings.hub_role; with hub_role off the records stay in the file and the held rows wait in the store (not refused); say so in the module doc.
  Test: hub_role off → no forward, no pair answers, status shows the pairs with whole = false reachable = false pending = 0.

F6 CONFIRMED (low): carrier_inbound accepts any pair (pair_of), so a tcp pair's CUSTODY_ACK is queued in fwd.acks for a pair forward_take refuses:
  up to 1024 sealed acks stay resident per tcp pair and PairView.pending is inflated. FIX: carrier_pair(&from)? at the top of carrier_inbound.

F7 PLAUSIBLE (low, security): replayed pair CUSTODY_ACKs are acted on over tcp. on_sealed counts a stale frame but still dispatches CustodyAck
  with no attribution check, and on_forward_ack acts on it: on a plain pair link an observer replays B's REJECT_FULL for row R on every retry, A
  drops R's pending entry and ignores B's real ACCEPTED; a replayed REJECT_UNENROLLED breaks the pair for the session. Also the held branch of
  on_forward_ack commits an ACCEPTED from ANY pair before the target == sender check. FIX: in the pair arm of on_custody_ack ignore a frame whose
  freshness is Stale (as carrier_inbound does) and require the connection to be attributed to the sender; move the target == sender check above
  the held branch (an ack from a pair that was not sent the put changes nothing). Test with a replayed ack.

F8 PLAUSIBLE (low): pump_hub's new-item path has no fallback without the TLV: if a dest's hint appears after the message was planned, plan_mtu
  reserved only CUSTODY_WRAP, seal_data(room - 131) returns None for full-size fragments and the fragment is skipped silently forever; only the
  stored path retries without the TLV (PLAN_RESERVE hides it on tcp-lan). FIX: when seal_data returns None with a hint, retry with room -
  CUSTODY_WRAP and send that put with dest_hub = None (a locator only).

F9 NOTE (optional): forwarding_evident reads the whole contacts table on every all-held transition for a hinted dest, on member nodes too. If an
  in-memory structure the fabric already keeps carries the hub role bits, use it; otherwise leave it and say so.

ALSO (lead): keep the M4 readings the board row records (the pair link-up ping, refusals ending with a tcp connection, REJECT_FULL retry triggers,
the forwarding reason as built) unless a fix above needs otherwise; keep hub_flow.rs unchanged; keep every existing forward.rs test passing or
rewrite it to the new design with the same claim.
`

const T = {
  M4b: { model: 'opus', effort: 'xhigh', brief: `TASK M4b (row "| M4b |" in the Wave 0 table of ${ROOT}/7-relay/TASKS.md): fix the review findings on
M4 (row "| M4 |", landed at 841d90a; its as-built names are in that row and in the module doc "Pairing and cross-hub forwarding" of M/fabric.rs; the
Layer 7 block starts at the comment "// Layer 7: pairing, the gate's forward branch, the forwarder, the hand-off"). Files you own:
${ROOT}/5-messaging/crates/mesh111/src/fabric.rs, ${ROOT}/5-messaging/crates/mesh111/src/lib.rs (re-exports), ${ROOT}/5-messaging/crates/mesh111/tests/forward.rs,
and in node111 only ${ROOT}/1-bootstrap/crates/node111/src/daemon/fabric/settings.rs (the paired_hubs row: the two new bools) and whatever single
line in ${ROOT}/1-bootstrap/crates/node111/src/daemon/fabric/mod.rs or json.rs a changed mesh111 shape forces (list each). Nothing else.
${FINDINGS}
Order of work: F1 and F5 (the record and the gates) → F2 (the forwarder's data structures and the window) → F3, F4, F6, F7, F8 → tests → the module
doc and the doc comments (every §x.y claim true for the code as it now is) → the full check.
CHECK (all green, paste the tail): from ${ROOT}/5-messaging: CARGO_TARGET_DIR=${SP}/target/mesh111 sh scripts/check.sh (musl SKIP); then
cargo test -p mesh111 --test forward --test hub_flow three times in a row; then from ${ROOT}/1-bootstrap: CARGO_TARGET_DIR=${SP}/target/node111
cargo test -p node111 --lib fabric; cargo clippy -p node111 --all-targets -- -D warnings; cargo fmt -p node111 -- --check.
NOTES must list, for the lead and for Wave 1's D6/D11/M5 facts: the exact new names (offer_pair, the PairedHub fields, FORWARD_RESIDENT, anything
renamed), the settings-file keys, and every behaviour that changed from M4's board row.` },
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
const F = await run(['M4b'], 'Fix')
log('fix: ' + F.map(r => `${r.id}=${r.status}`).join(' · '))
if (a.stopAfter === 'fix' || !F.length || F[0].status === 'blocked') return { F }

phase('Re-review')
const R = await agent(`You are a read-only adversarial reviewer of one Rust change in ${ROOT} (the 111network mesh OS). Do NOT run cargo, do NOT edit any
file, do NOT start any process; read only. The change: the uncommitted working-tree diff of 5-messaging/crates/mesh111/src/fabric.rs, src/lib.rs,
tests/forward.rs and 1-bootstrap/crates/node111/src/daemon/fabric/settings.rs (run: cd ${ROOT} && git diff --histogram -- <those paths>; the
default diff misaligns one untouched section). It is task M4b, the fix of these findings on the earlier change (read them first, they are the
claims to verify):
${FINDINGS}
The author's own report: ${JSON.stringify(F[0]).slice(0, 6000)}
Spec: ${ROOT}/7-relay/README.md §4.6, §5.1-§5.9, §13.1; Layer 5 amendments in ${ROOT}/5-messaging/README.md. Verify: (1) each finding is actually
closed by a real code path (trace it, cite file:line in the CURRENT file); (2) no Layer 5 no-pair behaviour changed; (3) the forwarder's work per
trigger is now bounded independently of the number of rows held (show the loops), the resident window really pages, and nothing resident is
unbounded; (4) a pair is admitted to any forward path only when whole = ours && theirs, persisted; (5) the tests exercise the claims (a pairing
with a former member, hub_role off, the once-bound of rule (1), the replayed ack, the window). Report only what you can show with a concrete
path; CONFIRMED when traced end to end, PLAUSIBLE otherwise; list what you checked and found sound and what is not covered.`,
  { label: 'L7:M4b-review', phase: 'Re-review', schema: REVIEW, model: 'opus', effort: 'high' })
log('re-review: ' + (R ? `${R.findings.length} findings` : 'none'))
return { F, R }
