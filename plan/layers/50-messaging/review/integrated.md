# Layer 5 — integrated review (V4, 2026-10-19)

Judge: fable (one agent on the running system: the three netns harness runs, the hosted-panel lint, the full
bootstrap gate, then the as-built paths read with an attacker's and a phone user's eyes). Tree: `b0f8c52` (polish wave
+ harness fix). Law: `50-messaging/README.md` v1.2 + as-built amendments, D18–D21, D23/D25 for what Layer 6 takes
over. Build record: `50-messaging/TASKS.md`, the triages under `review/`, the run journals under `runs/journals/`.

## 1. Verdict: **fix-then-lock**

Every gate is green on this tree and the security posture holds up under the paths I could exercise (sealed envelope
and counters, hub custody with one copy and exactly-once delivery, INTRO requests, the enroll end/abandon paths added
today, the tcp-lan exposure rules, the panel guards with the owner-accepted residual, the Meshtastic service, the
loopback-only bearer, the installer pins). One finding is major and demonstrable, and it is a liveness defect in the
layer's one promise that is "non-negotiable" (delivery): after a hub's **unclean** restart (power loss on a Cat-4 hub is
the normal failure), the L2 unclean-reboot jump moves every persisted member's receive window ahead of the member's
live send counter, so the member's next packets verify but are judged *stale*; the hub then attributes nothing to that
member and silently ignores its `MAILBOX_FETCH`, `CUSTODY_PUT` and `ENROLL` until up to 32 further packets have been
sent — with "no timers", that is many reconnects. It is the mechanism behind the lead's 1-in-3 step-13b timeout. The
fix is small and lives in `id111` (one verdict rule plus a test); nothing else in the layer needs redesign. Lock after
that fix and the two minors tied to it (m1, m3) land and the three-node smoke runs green three times in a row.

## 2. Gate runs (this box, user namespaces, no hardware)

| # | Command | Result | Duration |
|---|---|---|---|
| 1 | `bash 40-interface/verify/smoke.sh` | `SMOKE GREEN — 118 checks ok, 0 FAIL` | 118.5 s (incl. UI + cargo build into a fresh target dir) |
| 2 | `bash 40-interface/verify/smoke.sh --three-node` (SMOKE_KEEP=1) | `SMOKE GREEN — 158 checks ok, 0 FAIL` | 38.2 s in the namespaces |
| 3 | `bash 40-interface/verify/panel-lint.sh` | `PANEL-LINT GREEN — 144 checks ok, 0 FAIL` (strict default) | 46.6 s |
| 4 | `10-bootstrap/scripts/check.sh` | `ALL GREEN` (818 cargo tests across the suites, UI 1192, G2 run on musl-x86_64 + link on aarch64, notices current, gate self-tests, budget eager 103.0 / lazy 188.1 KB) | 695 s |

Material notes (read as evidence, not proof):

- Three-node: the corrected C expectation holds (C NOT SET until its first sealed packet; LIVE `peer_linked` after A's
  ACK), H counts exactly 2 attributed peers after C's refused enrolment (the dial was released), C has no hub card
  (`GET /contacts/H → 404`). Step 13b passed in my run (`A fetched it from the restarted hub by cursor → delivered`),
  but see Major M1: the pass depends on how far each member's counter had run ahead of the hub's last batch commit.
  The lead's failing run (`scratchpad/three-node-2.log:149`) shows B stuck at `relayed` while A's thread already
  holds the item `delivered` — exactly the half of the receipt path M1 predicts. Harness notes I accept: fragment
  count read off the wire (no API exposes it), `store_full` not forceable (the custody quota stands in), orphaned
  children after `kill -9` reaped by the harness (m4).
- Panel lint: against the placeholder bundle (the real bundle is an external gate); RESIDUAL #2 printed twice
  (`lan/probe.stealth-get`, `lan/probe.dashboard-frame-get`) as the owner accepted and §13 now states; one more note
  worth keeping in view: all 17 panel-origin requests carried the dashboard's `s111` cookie (cookies ignore ports) —
  HttpOnly, never forwarded by the static/shim server, stripped by the reserved proxy; a Layer 6 seam (§7).
- Nothing I started is left running; the kept three-node root and the lint scratch were read and removed.

### 2.1 Full gate

`ALL GREEN` after 695 s in a fresh target dir (`scratchpad/v4/check.log`): every cargo stage of node111 and mesh111
(fmt, clippy, tests, musl ×2), shellcheck, notices + generator tests, the hermetic build-script gate, systemd-analyze,
G2 (SQLCipher static link + run on musl-x86_64; link on aarch64 — the run there is the external gate), the presence
sidecar (tests + lock structure), the UI (1192 tests, D21 budgets). `REQUIRE_G2=1` self-test passed. All four gates
are green on this tree; the lock condition fails only on M1 below, which no gate exercises deterministically.

## 3. Majors

### M1 — After an unclean restart a node attributes nothing from a persisted peer for up to 32 packets; the hub mailbox is silently dead for that member

**Where.** `20-identity/crates/id111/src/counters.rs:664–679` (`apply_unclean_jump`: every persisted peer's receive
high-water `+= J`, `J = batch_n = 64`, bitmap cleared since `J ≥ 32`) and `:547–581` (`observe_counter`: Stale for any
counter ≤ `high − 32` or already seen). `50-messaging/crates/mesh111/src/fabric.rs:2447–2451` attributes a connection
only on `Freshness::Fresh`; `:2478–2497` answer `CUSTODY_PUT`, `MAILBOX_FETCH` and `ENROLL` only on an attributed
connection; `:2753` pushes only to attributed connections that fetched. The member's counters toward the hub are
batch-committed too, but the member was not restarted, so they continue from the live value.

**Mechanism.** The hub's file holds each member's `recv_high` as of its last batch commit (every 64 operations across
all peers), at most 63 behind the live value. After `kill -9` the hub reopens with `high = persisted + 64` and an
empty bitmap, so it accepts only counters in `(persisted + 32, persisted + 64]`. A member whose live counter was
within 32 of the persisted value sends `persisted + k + 1 … persisted + 32` next: each verifies (`pair_mac`), each is
offered to the window, each is **Stale**. Content still flows (A11: `msg_seq` kinds are processed), but attribution
never happens, so the hub ignores the member's FETCH (no items delivered), its CUSTODY_PUTs (its messages stay `sent`
for ever, and `custody_inflight` keeps the member from retrying on that socket), and a re-ENROLL (the route answers
"the hub did not answer" after 20 s and abandons the attempt). The member's own view says the opposite: `hub.reachable`
is true and the `fabric` row reads LIVE "Your home hub is reachable; messages are delivered or held for you" (m3).
With no timers, the member only produces packets on link events (FETCH + COMMIT per reconnect), so the stall lasts
dozens of reconnects or many sent messages; a clean member restart does not help (no send-side jump). The same rule
hits an accepted (inbound) direct peer after *this* node's unclean restart: its packets are Stale here for ≤ 32
packets, and this node sends to it only on an attributed or self-dialed connection.

**Demonstrate.** (a) Deterministic, against `id111` as shipped:
`scratchpad/v4/jumpdemo/src/lib.rs` (a 25-line test): persist a peer at high 10, observe 11..15 unpersisted, drop
without `shutdown()`, `open()`, then `observe_counter(16)` → **Stale**; the first Fresh counter is 43 — 27 legitimate
packets judged stale (output: `after the unclean open: high=74 (persisted 10 + J 64), window accepts (42, 74]`).
The repo's own test `counters.rs:946 unclean_reboot_jumps_send_and_receive_sides` asserts the same window arithmetic.
(b) In the harness: step 13b (`smoke.sh:1238–1258`) with any member whose counter is close to the hub's last commit;
the lead hit it once in three runs, B stuck at `relayed` with A `delivered`. (c) Spec reading: §4 and §8.1 say "a
replayed packet verifies but is never fresh (L2's unclean-reboot jump covers the last-N replays)" — the jump was
reasoned about as a replay guard for the *crashed receiver's* last N packets, not for its effect on live members.

**Fix (one sentence).** In `id111`, keep the J jump for the §6.5 `on_receive` rejection but let `observe_counter`
(A11) report Fresh for any *unseen* counter above the peer's high-water as persisted at the unclean open (record it in
`apply_unclean_jump`), with a test; the stated residual is then that one of the last ≤ N pre-crash packets, replayed
after the crash, can attribute a socket for pushes of sealed items it cannot read and for card answers (the same
bounded residual §6.5 already hands to Layer 5 idempotency). Alternative at the fabric: on an inbound connection
whose first announce claims the packet's verified sender, attribute on Stale when the counter is above `high − J`;
same residual, one crate further from the window. Either way, re-run the three-node smoke three times.

## 4. Minors

- **m1 `Fabric::retry_all` is never called.** The mesh111 module doc (`fabric.rs:38–43`) and `tests/lossy.rs:47` say the
  daemon calls it on fabric events; `grep retry_all 10-bootstrap/crates/node111/src` finds no caller. So a member
  re-FETCHes only at link-up or enrolment, and `custody_inflight` on a live socket is cleared only by a disconnect;
  `Fabric::retry` (`fabric.rs:3085`) clears outstanding fragments but not the message's inflight custody entries, so
  the UI's retry does nothing for a CUSTODY_PUT the hub ignored (M1) or lost. Fix: call `retry_all` from the daemon's
  fabric-event pump as documented, and have `retry` drop that message's `custody_inflight`/`custody_pending` entries.
- **m2 `leave_hub` releases a shared dial.** `fabric.rs:3396–3403` releases the hub's address even when `peer_dials`
  holds it (recorded by F2P as a gap). Fix: `if !self.peer_dials.contains(&addr)` before `d.release`.
- **m3 "hub reachable" is self-judged.** `fabric.rs:3427+` sets `hub.reachable` from the member's own sendable
  connection and `services/mod.rs:1164–1169` turns that into LIVE copy promising delivery; no hub-authenticated
  answer is required. With M1 the row lies. Fix: reachable only after an authenticated hub packet on the current
  connection since link-up (ENROLL_OK, MAILBOX_ITEM, CUSTODY_ACK), else NOT SET "connected to the hub, waiting for its
  answer".
- **m4 Children outlive `111d` after SIGKILL** (V1v note, `smoke.sh:1267–1273`: 111-agent, the model, meshmud and the
  reader keep running; no `PR_SET_PDEATHSIG`). Layer 1 file. Fix: `prctl(PR_SET_PDEATHSIG)` in the child `pre_exec`, or
  state the cgroup-kill reliance in the Layer 1 README and the Termux runbook.
- **m5 Reader keep-alive can block.** `meshtastic_presence.py:498–511` `session_keepalive` does a blocking `sendall`;
  if meshmud stops reading, the loop blocks (HB residual). Fix: a write timeout on that socket, timeout ⇒ "link lost".
- **m6 Announce before hello.** `tcp_lan.rs` `serve` writes hello + the own announce before reading the peer's hello:
  any TCP connector on the LAN harvests the NodeId and keys of every hub / LAN-sharing node. Spec-conformant (§8.1,
  §18.4 cleartext announces) — record it as a stated residual, or read the peer's hello first (no cost, still
  symmetric between 111 peers).
- **m7 gen-manifest listing check compares names, not hashes** (`10-bootstrap/manifest/gen-manifest.py:211–239`;
  MANP gap). `verify_tree` at stage time refuses a wrong hash, `sha256sum -c` is the human step. Fix: compare each
  listed sha256 with the tarball member (`read_tar` already computes them).
- **m8 Stale docs.** `10-bootstrap/README.md` "Artifacts per release" still lists only `bin/111` + `share/kb/`;
  `LICENSING.md` §8 still says the release checklist does not record the source offer (DOCS deviations). Two one-line
  edits.

## 5. Deviations I accept, and why

- Read marks in memory only (F1): v1 scope; a `read_seq` column is a recorded wish.
- "Ignore" = client-side fold, "forget request" = DELETE behind confirm (U1, lead call): matches §7.2 (kept, never
  auto-pruned, no ACK) with no contract change.
- Bearer callers never mint panel tickets (P1 m2): the agent opens no panel; §18.9 holds.
- `DUPLICATE` from the home hub counts as `relayed` (F1 reading): same fact §9 states.
- Manifest `pkg`/`bundle` names parse-and-ignore, closure enforced in `verify_layer5_tree` (B1 m6): owner confirmed
  the reversible call; call sites ×3 verified (`update_helper.rs:185`, `updater/fetch.rs:174`, `install/stage.rs:41,86`).
- A timed-out enrol wait abandons the attempt and a late `ENROLL_OK` is ignored (F2P): the clean reading of "leaves
  nothing behind"; covered by `hub_flow.rs:599,651,692`.
- P1g "Origin decides when present" (a matching Origin with a foreign Referer passes): a browser cannot produce that
  pair cross-origin; the lint's probes are refused.
- Residual #2 (header-less same-site GET on a plain-HTTP LAN origin): owner-accepted, stated in §13.
- VENV: `venvs/presence` as a link to a complete build + status file + flock; the TOCTOU residual (a compromised
  `node111` swapping `presence.d`) is inside the trust boundary the installer already crosses.
- HB: the ToRadio allow-list of exactly `want_config_id` and a nonce-0 `heartbeat`, guarded at both library layers.
- Harness honesty gaps (fragment count off the wire, `store_full` unreachable, counters now exposed): recorded.
- Tapes carry a host-dependent `opencode` path in the codeagent row: cosmetic; a lead call.

## 6. External gates that stay listed before the first signed release

- meshmud with a real Meshtastic board: serial open without an unwanted reset, config fan-out to a second TCP client,
  the shim's per-session downlinks; Termux build of meshmud (TCP-to-board uplink) and of the presence venv.
- The real Meshtastic web bundle under `panel-lint.sh` (`PANEL_LINT_BUNDLE`): its actual off-origin hosts versus
  `[offline] optional_hosts`, inline handlers/service workers, the connect-dialog prefill.
- The presence venv built from a real pip/PyPI closure (tests use fakes); post-promotion wheel availability (the
  rebuild fails closed to the previous venv without them — owner question still open).
- aarch64 store open/write/prune in an Android emulator (or recorded as external where CI has none); Go toolchain in
  CI for `meshmud-build.sh` (license verdict file).
- Payload assembly and signing never run end to end (no `release.sh`; the checklist's steps were exercised on scratch
  artifacts only).
- Two-board MeshCore bench for the `kiss-modem` seam (Layer 6).

## 7. What Layer 6 inherits (seams it must not break)

- The `Link` trait, `PathProfile`, `LinkEvent::{Connected, Disconnected}` and `Dialer` (§8): a driver reports
  connections and announces and **never attributes** — attribution stays the fabric's (`on_sealed`), and after M1's fix
  the verdict rule lives in `id111` alone; drivers add no replay logic of their own. `Link::send` must never wait on
  its own `recv` (the bounded frame queue is the back-pressure contract).
- The custody/mailbox wire (kinds 0x03–0x09, `pkt_ref`, hub-local cursors, `REJECT_UNKNOWN_DEST` ⇒ `hub_not_yours`):
  hub-to-hub replication (A+) extends it with summary vectors, it does not change it; the 160 B default profile and
  `PLAN_RESERVE` govern small-MTU hub links.
- The `Sealer` boundary: the keyset never enters mesh111; derived pair keys do; `forget_pair_key` on forget. Restore-
  pending means no link, no announce, no counters write.
- INTRO is tcp-lan-only; LoRa first contact needs a card (card-in-fragment-0 is the recorded design).
- `devices.json` Meshtastic binding → `interfaces.toml` (born in Layer 6); the port-owner table `daemon/ports.rs`
  (`409 port_busy`); identify profile swaps in place (never a second open).
- meshmud's TCP downlink `127.0.0.1:11230` is the only path to the board: D25 F2's Rust `meshtastic` client is one
  more downlink client with the reader's allow-list mirrored (text to a channel/node only; never admin/config; no
  heartbeat with nonce ≠ 0); the presence reader retires only after parity, and its file hardening
  (`meshtastic/presence.rs`) stays for any file-based feed.
- Panel guards and header policy (§13 as built, incl. the accepted residual), the ticket → cookie lifecycle, and for
  the reserved proxy (MeshChatX panel): strip `s111`/`p111_*` from upstream requests, drop upstream `Set-Cookie` named
  like ours, `Domain=` stripped, no CORS headers ever — the lint note that `s111` rides to every panel origin is why.
- The bearer rule (loopback peer + the per-spawn token, reads only) and the proposal-gated write path for every new
  agent tool (`transport.read` reads; sends propose).
- `verify_layer5_tree` + manifest pins for every new payload asset; `Status.layers` adds a layer only when it runs;
  `FabricStatus.counters` are diagnostics, not contract.
- Store schema V3 is additive (D25 F1) and prunes under the §10.4 order; `store_full` semantics unchanged.
