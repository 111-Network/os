# Layer 5 — Wave 2 review triage (2026-10-14/15)

Review run `wf_938461a1-1cb` (script `layers/05-messaging/runs/workflows/wave-2-review.js`): F2 by fable (xhigh), B1 and U2 by opus.
Full findings: the run's `journal.jsonl`. Commits reviewed: B1 `09dc397` (+ notices `43c98bd`) · U2 `282bff8` · F2 `7554926`.
Every finding is accepted unless marked; bounces run as fresh agents (one per task, disjoint files); the lead re-runs
each check, reads the diff and commits per bounce. Wave 3 (V1 smoke, V2 panel lint) runs in parallel; its gaps join the
same bounce round or a second one.

| Task | Verdict | Majors | Minors | Deviations |
|---|---|---|---|---|
| F2 daemon wiring | fix-then-lock | 1 | 9 | 13 accepted |
| U2 UI Panel/Nearby/Settings | fix-then-lock | 1 | 11 | 10 accepted |
| B1 build/gates | fix-then-lock | 0 | 9 | 11 accepted |

## F2 — bounce (all)

- **M1 restore-pending becomes a counter reset.** `shutdown_counters()` (clean stop and the `boot()` error path) persists
  `announce_last = 0` while id111's `CounterStore` is `restore_pending()`; the next start mints counter 1 / epoch 2 and the
  node's `msg_seq` epochs restart below every peer's prefix (silent loss) or the store refuses `EpochNotIncreasing`. Fix:
  never persist while restore-pending (both call sites); while restore-pending register no tcp-lan, bind nothing, maintain
  no peers, keep `own_announce()` empty so offer/card keep answering 409; **and** the matching guard inside id111
  (`CounterStore::shutdown/commit/persist` refuse while pending — identity crate, additive, with a test and a vectors note);
  test: drop `counters`, start with a stored announce, assert no tcp-lan, shutdown writes no file.
- Minors m1–m9: dial targets refuse unspecified / multicast / broadcast (and loopback under `refuse_loopback`) in the route
  and in `settings::decode` · enroll persists `home_hub` synchronously (`on_settings` before answering) · a corrupt or
  oversize `fabric/settings` is moved aside (`settings.invalid-<n>`), logged once, and `home_hub` is rebuilt from the single
  `enrolled` contact · CLI: a bare enrol code is read from the TTY, never argv (a scanned `111HUB:` bundle may ride argv);
  `dechunk` byte-safe · **contract regen pass (F2-bounce owns openapi + gen):** `offerHub` 409 (`restore_pending`,
  `bad_request`), `getOwnCard` 409 `restore_pending`, `enrollHub` 502, `decideProposal` outcomes, `FabricStatus.store.
  restore_pending` (mesh111 `StoreView` has it; `json.rs` drops it), the quota-classes description (refusable classes
  only), and **the lead's additive `cancelMessage`** — `POST /messages/{id}/cancel` → 202 for an own `queued` message (spec
  §4: cancelled before it leaves ⇒ empty tombstone; `reason_code: cancelled`), 409 otherwise · applied-proposal guard
  survives a restart (decision file written first with `applied: {pending: true}`, finalised after the send; a crash leaves a
  visibly half-applied proposal, never a double send) · announce re-minted on a `Settings` event (adapter `set_announce`,
  additive one-liner in mesh111 `link/tcp_lan.rs`) · restore-pending doc sentence corrected and the `fabric` service row
  names the state (`identity restore floor missing — run …`) instead of "store could not be opened"; report which Layer 2
  command sets the floor today (owner question if none) · tests: restore-pending + stored announce + shutdown; one node with
  `refuse_loopback` on; two-node `peers` with `listen_lan`.

## U2 — bounce (all)

- **M1 panel blocked by the page's own CSP.** `frame-src` is fixed when the document loads; a panel enabled afterwards (the
  usual first-run order) is refused by the parent and the frame shows the browser's blocked page. Fix: a
  `securitypolicyviolation` listener in the frame host — when the blocked origin is the panel's origin, replace the frame
  with honest copy ("this page was loaded before the panel was switched on … reload") and a reload button; keep Open full
  client; warn ahead for a panel that was not enabled+listening at the first `GET /panels`; test the branch.
- Minors m1–m11: read `FabricStatus.store.restore_pending` (optional until the regen lands; say "not reported" meanwhile)
  and reword the restore copy to the §15 as-built (new messages refused until the floor is set) · quota rows built from
  the node's `settings.quotas` keys with plain labels (no `msg` / `agent_op`) · a one-time code clears when
  `hub.offer_active` drops · `ticketSrc` also requires 11190 ≤ port ≤ 11229 and port ≠ the dashboard's · mint reply shape
  checked (`url` non-empty string, `expires_in` clamped 1–300, `onError` in `fresh()`) · hub name isolated and clipped ·
  prune / store-full copy per §10.4 (4) in full · §13 "dashboard re-mints" and §14 "panel connection" copy (node address
  with copy button; "session expired → Reload"), recorded in the README note · Nearby: future clock hints read "clock ahead
  of this node"; `starting` marks the list last-known · stale hub card cleared on a bare-code paste · tests: hostile
  vectors through status/service fields, one full pre-U2 prompt string pinned, a crate-generated `111HUB` vector, the CSP
  branch · **carried from F2/U1:** the mock's enroll bundle becomes `111HUB:<card>:<code>:<address>` (`mock/messaging.ts`),
  and the thread view gains a **cancel** control for own `queued` messages (calls the new `cancelMessage`; optional until the
  regen lands).

## B1 — bounce (all)

- m1/m2 gate honesty: assert strings only SQLCipher's C code carries (`cipher_plaintext_header_size`) and `OpenSSL <ver>`
  (version from Cargo.lock's `openssl-src` suffix) on the release binaries and on the aarch64 test binaries; INTERP/NEEDED
  checks on x86_64 too; `unset` the pkg-config / `OPENSSL_NO_VENDOR` / `OPENSSL_DIR` escape hatches in cross-build.sh ·
  m3 the same `CC_<triple>` / `AR_<triple>` exports in both check.sh files; messaging's musl block gated on both compilers;
  a skipped G2 ends "GREEN (G2 SKIPPED — not a lock run)" never "ALL GREEN", and fails under `REQUIRE_G2=1` · m4 **wire
  `verify_layer5_tree`** at the three Layer 1 call sites (update_helper.rs after `verify_tree`, install/stage.rs after its
  `verify_tree` and at the :30 content probe, updater/fetch.rs after `stage_payload`, removing the staging dir on error) with
  tests; fix the manifest.rs and gen-manifest.py wording · m5 gen-manifest cross-checks each `payload-<triple>.tar.gz`
  against the pins it writes (bin/meshmud, the bundle tree recomputed from the .tar, the presence lock) and refuses a
  `meshmud` triple without a payload · m6 **lead decision (reversible, owner may flip):** parse-time closure of `pkg`/`bundle`
  names is dropped — unknown names parse and are ignored (the pre-B1 forward-compatible behaviour; the manifest is signed
  and `verify_layer5_tree` still refuses unpinned assets); `manifest-version` stays 1; the L1 README wording (`pkg.*`) stays
  true; a reader-version evolution path is an open owner question before the first signed release · m7 lstat every path
  component between `tree` and each target; check `panels` before hashing; symlinked-`bin` test · m8 the two tool-dependent
  gate tests fail or skip visibly · m9 notices: SQLCipher's `sqlcipher/LICENSE` verbatim with "Copyright (c) 2008-2020
  Zetetic LLC"; OpenSSL line "OpenSSL 3.6.3 — Copyright 1995-2026 The OpenSSL Project Authors. All Rights Reserved. Apache
  License 2.0"; regenerate.

## Carried / open

- Owner questions: manifest evolution path before the first signed release (B1 m6); which command sets the Layer 2
  restore floor (F2 m8); release-checklist and presence-venv build-step rows (B1 Wave 2 deviations 5–6).
- Identify button on the device page (no owner yet; agent-guided path works) — Layer 6 or a small UI task.
- V1/V2 gaps (Wave 3, in flight) join the next bounce.
