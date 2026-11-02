# Layer 9 — Self-Healing & Evolution: options brief

Status: 🧭 deciding — awaiting the owner (sweep complete, run ids in research/coordinates.md) · lead-written from the ten reports in `research/`
(`00-code-seams.md` line-read on `a65ac6d`; `01`–`05` web-checked where this box's network policy allowed: package indexes, crates.io's API and raw
GitHub files only; arXiv and most documentation sites were denied, so every paper figure below is a search snippet; the critic's `06`; three gap-fills:
`07-gap-1` the hub code-fix pipeline, `07-gap-2` `AGENT_OP` delegation, `07-gap-3` playbook rules). Every code and spec line a pick rests on was re-read
for this brief. Reads with: `os/README.md` §9, the category table and the principles · the hand-offs in [`README.md`](README.md) · D10, D17, D20–D22,
D26, D29–D32 · `AGENTS.md` (the Pi floor) · Layer 8's sweep (`../8-storage/research/`): F3 is decided jointly with it.

Owner scope: Layers 8 and 9 started together on 2026-11-02 (owner: "start layer 8 and 9 research in parallel now and document all"). `INDEX.md`
"Process (per layer)": research → this brief → the owner's decision (`DECISIONS.md`) → spec `9-evolution/README.md` → adversarial panel → build.
**No Layer 9 code before the owner decides.** Layer 7, the layer below, locked its spec v1.0 today (D32); it is cited by section, never by line.

The Pi floor (`AGENTS.md`, verbatim): "No task may add a resident process, a periodic poll, a runtime dependency or a build-time weight to the required
configuration without the owner's say; measure (RSS, binary size, listeners) and record it in the layer's `TASKS.md` when it changes." The required
configuration today (release build): `111` 19,404,064 B, `111-agent` 4,392,280 B, idle RSS 19.7 MB and 5.3 MB, listeners `:11180` (+ best-effort `:80`) and
`:11182` (`6-transport/TASKS.md:82`). Three words used below: an **incident** is one recorded failure or degradation; a **playbook** is a checked procedure
for one kind of incident; a **typed action** is one entry of a closed list of changes the daemon knows how to apply, probe and undo.

## 0. What the research settled (facts that bound every option)

1. **The observer belongs in `111d`.** Every event source Layer 9 needs is already an in-process channel there: each child's `watch` of `ChildState
   {Running, Backoff, Degraded, Stopped}` (`N/supervise.rs:415-421`, `state_watch()` `:627`), the dashboard `EventBus`, the netlink and uevent sockets, the
   fabric's computed facts. Subscribing costs no timer. `111-agent` is itself a crashable child whose tools cannot read the services registry, the service
   logs or the update state (00 §1, §3.1 (a)–(b); 01 §3.1). (`N/` = `1-bootstrap/crates/node111/src/daemon/`, `A/` = `3-intelligence/crates/agent111/src/`.)
2. **The supervisor observes too little.** `let _ = child.wait().await` drops the exit status (`N/supervise.rs:972`); an ordinary exit logs nothing; the
   default policy marks a child `Degraded` after 5 exits in 60 s (`:61-71`) and then waits for a restart request forever (`:996-1009`). So Layer 3's
   "restarts a dead child ≤10 s" (`3-intelligence/README.md:51`) holds for the first four exits only (00 trap 1).
3. **Diagnosis today is one tool-less model call.** `diagnose`/`propose` send the daemon's health, the agent's own 30-event journal and up to eight 8 KiB KB
   refs under a system prompt that says "You have no tools" (`A/tasks.rs:320-345`); the result is unsigned Markdown written with `std::fs::write`, not
   atomically (`A/store.rs:120-146`). Accepting a `diagnose`, `propose` or `patch` proposal only records `{decision, at}`; nothing acts on `decisions/` (00 §3.1 (e)).
4. **The Layer 6 applier is the template, and its undo lives in memory.** Pending record first, one proposal at a time, per action snapshot → apply → probe
   (1 s steps, ≤30 s) → keep or revert; four applied actions, two one-shots, four refused names (`N/devices/applier.rs:20-40,83-104`). The snapshot is an
   in-memory struct (`:835-839`), and "a crash mid-apply leaves the pending record on disk: the proposal reads decided and is never applied a second time"
   (`:35-37`): the change stays and is never reverted. Nothing watches a change after it is kept (02 F1; 05 F1).
5. **The one armed record that survives a restart is the update probation journal**, written by root before the flip and resolved by `update-guard`
   before the next start (`1-bootstrap/crates/node111/src/probation.rs:7-17`; 02 F2). Its deadline is wall-clock epoch seconds (00 trap 10). The daemon
   confirms after 60 s of sustained green inside a 600 s deadline (`N/mod.rs:133-176`, `probation.rs:57`); `1-bootstrap/README.md:158` says "the 60s probation window".
6. **A node validates only at runtime**: the health predicate, the services compute, the applier probes, probation, the payload and KB hash checks.
   Anything that tests code needs cargo, git, user namespaces and jq, and runs in CI or on a dev box (00 §3.1 (d)). On a node, a "fix" is a typed
   reversible action or a signed release.
7. **Code reaches a node only as a release.** The hub origin serves the signed manifest, its signature and pinned blobs, nothing else (L7 §9.1); git is
   rejected on the node (D30 F6); the KB ships inside the root-owned payload, so an on-node KB edit turns `kb` red (`1-bootstrap/README.md:39,169`; 00 §3.1 (f)).
8. **The hub's patch path is unsafe as built.** opencode allows every tool by default, bash and web fetch included (upstream docs and the v1.19.5 source:
   06 §2.1; 07-gap-1 F1), and the agent writes no `permission` block into `opencode.json` (`A/tasks.rs:490-508`). On Linux a loopback peer is trusted as
   the human (`N/dashboard/api_v1/auth.rs:3-10`), so a confused or prompt-injected model could decide a proposal with `curl 127.0.0.1:11180` and no bearer:
   a self-approval path, read in code, not tested (07-gap-1 §1). It also breaks "no host shell for the agent" (`3-intelligence/README.md:55`). A
   deny-all permission block closes it and works offline (07-gap-1 F2).
9. **The hub has no source tree to patch.** The payload is `bin/111`, `share/kb/` and assets. A hub source asset (`git archive` of the release tag) is 1.7 MB
   gzipped (measured). A full offline build is four lock graphs, 349 crates, 87.6 MB compressed, plus a C toolchain, make and perl for `node111`; the two
   pure-Rust workspaces need only a Rust toolchain (111–127 MB xz) plus 4.5 and 14.4 MB of crates. No build time for our workspace on a Pi 5 or an N100 is
   published (07-gap-1 F6–F9).
10. **The published evidence puts the checks outside the agent.** Frontier agents solve about 11 % of SRE and root-cause scenarios (ITBench, OpenRCA);
    the production pattern is hand-built handlers with the model only classifying (RCACopilot); validators are satisfied by deleting functionality (GenProg:
    2 of 105 correct; Kali, which only deletes functionality, matched it); a self-modifying agent fabricated a test log and removed its own detectors (the
    Darwin Gödel Machine); production auto-fix keeps a human gate (Google 15 % fixed, Meta 25.5 % landed). All search snippets (01 §3.4; 02 F14–F17; 03
    finding 11; 04 findings 10–18). No evaluation of a 4B model on incident diagnosis was found.
11. **The local model is slow on a Pi**: about 2.3 tok/s generation for a 4B Q4_K_M model on a Pi 5 (Ollama, search snippet, unverified); today's diagnose
    context (~6,000 tokens) is about 20 minutes of prompt evaluation at 5 tok/s (arithmetic, unverified); the model weighs 3–4 GB (01 §3.4; 05 F7). Layer 3
    gives that model to 2B, 4A and 4B as the post-install offer (`3-intelligence/README.md:27-36`).
12. **Layer 0 cannot hold a playbook today.** Recipes are `install-<project>-on-<device>.md` only (`0-knowledge/README.md:32`, rule N03), the frontmatter is
    closed at twelve fields (F02), `skills/` is deferred (`:143`). A `playbook` category with a TOML block under `## Details` needs no new field, and
    `build.py` (Python ≥3.11) lints it with the standard library's `tomllib` (07-gap-3 §3.6).
13. **Rule engines weigh more than the need.** `cel` 0.15.0 (MIT): about 11–23 new crates and +0.7–3 MB (estimates, unbuilt), pre-1.0, eight releases between
    0.13.0 (2026-03-12) and 0.15.0 (2026-10-06); `serde_json_path` 0.7.2 (MIT) selects nodes and has no substring test, counter or
    quantifier; a closed predicate enum is 0 crates, ~200 lines, about +10–30 KB (estimate). Every check the reports name is a typed read of state
    `111d` already computes (07-gap-3 §2, §3.1, §3.3).
14. **`AGENT_OP` (0x10) is a reserved byte.** SIGNED-mandatory, interactive priority, `Class::Agent` (`5-messaging/README.md:139-140,173`); no payload
    schema, no stream id, no send API, no consumer; an inbound one becomes a textless row in the sender's chat thread, as do `KB`, `UPDATE` and `PATCH`
    (07-gap-2 F1; `8-storage/research/00-code-seams.md` findings 17–19).
15. **Layer 3's LAN offload is not built on the hub side.** The hub's `llama-server` binds `127.0.0.1` with no key, the phone's model client sends no
    `Authorization` header, and the target is a private IP literal that fails off the LAN; "LAN binding + pairing auth" (`3-intelligence/README.md:48,110`)
    is prose (07-gap-2 F2–F3).
16. **`PATCH` (0x13) is signed, public-class and bulk.** Locked Layer 5 sends "larger objects and every bulk class" by Tier 2 (`OBJ_*`), built in v1.1
    (`5-messaging/README.md:239`): a v1 fix frame needs an additive Layer 5 rule or waits. A `tcp-lan` frame carries up to 65,445 B; Tier 1 ≈1.07 KB
    at the 160 B floor and ≈2.2 KB at 233 B for unsigned fragments (03 finding 1); `PATCH` and `AGENT_OP` are SIGNED-mandatory
    (`5-messaging/README.md:139-140`), and a SIGNED fragment carries 3–5 B at 160 B and 76–80 B at 233 B, so sixteen fragments hold about 1.2 KB at
    233 B and no Layer 9 op fits the flood floor (07-gap-2 F4).
    Public signed objects are stored plain (`:453`): a fix carrying incident evidence is never a public object.
17. **Signing is the identity holder's.** The agent holds **zero** key material (`2-identity/README.md:224`); every signed artifact carries its own ASCII
    domain tag, and identity keys sign "announces, proofs, and control artifacts only — never routine traffic" (`:85`); the tags live in one table (L2
    §13.1 A4). The delegation cert is pinned and refused by v1 verifiers; its expiry counts in mesh epochs (L2 §9), which Layer 7 pins as a day and
    evaluates for no consumer in v1 (L7 §8).
18. **The witnesses of `111d`'s own death exist and do nothing.** The Termux runit `finish` hook receives the exit code and signal and only logs
    (`1-bootstrap/termux/sv/111node/finish:8`); the 5-minute cron `/health` job appends to `health-failures.log`, which nothing reads (`termux/cron/crontab.line:5`;
    00 trap 9). On Linux the unit runs `Restart=always` with `StartLimitIntervalSec=0` (`1-bootstrap/README.md:129`), so `OnFailure=` never fires;
    `ExecStopPost=` sees `$SERVICE_RESULT`, `$EXIT_CODE`, `$EXIT_STATUS` on every stop (05 F5); the unit has none today (`systemd/111node.service`, grep).
19. **The timer baseline is counted, not measured, and the counts disagree**: about 265 wakeups a minute on a node whose agent is not running (00 §3.1 (a):
    Layer 7's research count of ≈205 plus the 2 s `status` tick, `N/dashboard/api_v1/status.rs:62`, the 2 s agent-relay poll, `N/dashboard/api_v1/relay.rs:111,892`, the 30 s LAN refresh,
    `N/dashboard/mod.rs:300`, and the Termux cron), about 100–120 (01 §3.1), ≈205 before and ≈114 after Layer 7's trim (L7 §3.6). Layer 7's no-timer gate
    measures it at build (L7 §3.7).
20. **Prompts are code.** The chat preamble and the task prompts are Rust constants (`A/chat.rs:37`, `A/tasks.rs:327,403`), the tool set is fixed (D10), the
    model pin is in the signed manifest: nothing about the agent changes without a release (04 findings 1–2).
21. **Bridging has a typed surface; the agent reaches a slice.** It proposes `bridge_edge` (enable) and `device` actions; "An edge's live counters let the
    agent suggest turning it off; it cannot do so" (`6-transport/README.md:458-459`; 04 finding 6).
22. **The APK hand-off has no data.** Three APKs are pinned in the manifest and installed by the host installer over `adb`; the running daemon never reads
    those pins and the receipt records none, so a phone does not know which APK versions it runs (07-gap-2 F10; `1-bootstrap/README.md:160`).
23. **Layer 8's research places Layer 9's state** in a `heal/journal` namespace of a node111 state service (`8-storage/research/04-local-store.md` §4 Fork 5 A),
    agent state in a second SQLCipher file only `111d` opens (`8-storage/research/07-gap-1.md` §4 pick A), no shared log in v1 with p2panda-core named as the
    adoption target (`03-shared-log.md` §4), and a hub-origin admission rule whose later pin sources include "a hub-owner-signed fix index"
    (`8-storage/research/07-gap-2.md` §3.6). Its critic flags Layer 9's plaintext incident file and its own hash chain (`8-storage/research/06-critic.md` conflict 9).
24. **The write path is human by construction.** The agent's bearer is refused on every non-GET (`auth.rs:927-933`, D26 (2)); a decision is made only inside
    `POST /api/v1/proposals/{id}/decision`; a `decisions/*.json` file is not an authority, since every 111 process runs as `node111` (00 trap 3).

## 1. What Layer 9 is, in one paragraph

Layer 9 makes a node notice its own failures, explain them, and fix the ones it can fix safely, with a human at the last step. `111d` records every
failure as an incident from events it already receives (no timer, no poll, no new process), checks it against signed playbooks without a model, asks
Layer 3's model only where one already runs and only to choose among the playbook's causes, and turns the answer into a proposal of typed actions. When
the human accepts, the daemon applies each action with a persisted undo, probes that the capability works, and reverts by itself on failure. A phone asks
its home hub. A validated typed fix can be sent, sealed, to the home hub or a pinned contact, where it is a proposal again; a code fix is a draft on a
hub that opted in, exported as a patch the human opens as a pull request. The agent improves only by release, the KB grows by drafts the human carries
upstream, bridging grows by typed actions. Nothing applies code, nothing is shared without a human, the agent never holds a key or writes anything but a
proposal, and Reticulum stays a separate program that Layer 9 reaches only through the existing applier and spine.

## 2. Polled forks (owner decides; recommendation first)

### F1. Autonomy in v1: what may happen without a human (critic fork 1; conflict 2; 00 fork 3; 02 fork D; 05 fork A)

- **A. Propose, with an automatic undo.** Every change is a proposal a human accepts (D26 (2) unchanged). The only things that happen without a human
  are the revert of a change that failed its own probe and the revert of a change that was armed when the daemon died (fact 4). The auto-apply rung is
  specified, not built: its grant record is pinned in the spec (written by a human session, naming an action class, a target and a count per boot; never
  a component, update, identity, consent or code item) so a later release adds it without redesign. Settings show the ladder observe-only · diagnose ·
  **propose** (default). Cost: zero resident; the undo is one file read at start. *(pick: 02 §4 D1; the critic's §2.2 reading; README §9 "Humans remain
  the final gate"; the principle "always human-supervised at the final step")*
- B. An opt-in auto-apply rung in v1 for typed reversible actions inside a user-set scope (05 §4.2: per layer × action kind, empty by default, a count
  cap of 3 when first enabled), including `111d` restarting a `Degraded` child (P4: "node-child restarts may skip approval but are rate-limited",
  `plan/cross-layer/parked.md:11`; `cross-layer/devices.md:194`: "rate-limited 1/5 min/service"). The critic reads D26 (2) as governing the agent's
  bearer, not a `111d` action inside a human-written grant (06 §2.2): **B does not reverse D26 but narrows how it reads; marked, the owner's call.** Cost: a
  scope table, a grant record, `decision.by = "policy"` in the audit, and a count cap the owner must square with README:79 (§5 Q1).
- C. Status quo, no undo automation: rejected; it leaves the crash hole of fact 4 open.
- In every option: a delegated reply (F11) and a fix from a peer (F10) never auto-apply; no rung applies code (structural, not a setting); the agent's
  bearer stays refused on every write. Amends: nothing under A (the grant format is new Layer 9 text); under B, P4's restart clause becomes spec and D26
  (2) gains one sentence ("a restart `111d` makes inside a human-written grant is not an agent write").

### F2. Observation: where the observer runs, how it wakes, who witnesses 111d's death (critic forks 2, 12a; conflicts 10, 12; 00 fork 1; 01 forks 1–3; 05 fork B)

- **A. Inside `111d`, events only.** One subscription per child `watch`, the `EventBus`, the netlink and uevent sockets, the fabric's computed facts and the
  start-up tombstones (`update.json` after a revert, the update-guard report); a detector is a function over state already computed; "sustained for N s"
  is a one-shot sleep armed by a transition, never an interval; one open incident per fingerprint, so a crash loop is one incident with a `count` and at
  most one model task (`TaskEngine::submit` has no cap and starts one thread per task, 00 §3.1 (a)). The supervisor publishes the exit status and a
  restart counter on its channel. The witness of `111d`'s own death is out of process and event-driven: an `ExecStopPost=` line in the Linux unit and the
  runit `finish` hook on Termux each append one line (code, signal, boot id) to a small inbox that `111d` reads at start, beside a run marker (`clean:false`
  at start, `true` on a graceful stop). **The Termux cron entry is removed** (conflict 12, settled for 05 against 01: its log has no reader and the finish
  hook sees every exit). Costs: +0 timers, +0 processes, +0 listeners; about 64 KiB for 64 open incidents (01 §3.2, estimate); two small writes per daemon
  run; on Termux a saving of one `curl` process every 5 min. Records without a model or an agent, so it serves 2A and 3 too. *(pick: 00 §4 fork 1; 01 §4
  fork 1 A; 05 §4.3 B)*
- B. A plus an opt-in deterministic probe every 15 min (disk, update staleness, battery on Android): +1 wakeup per 900 s on nodes that turn it on, no model
  call. Not in v1; if wanted later, one re-armed one-shot inside `111d` on hubs, never a second cron entry.
- C. A literal monitor loop (every 60 s, ask the model "anything wrong?"): 1,440 model calls a day, the 3–4 GB model never sleeps. Rejected, with the
  resident monitors the sweep weighed: monit (AGPL-3.0, verified; a poll loop and a second supervisor), Netdata (GPL-3.0 agent, non-free UI part,
  telemetry on by default, ~30–100+ MiB per snippets), Prometheus + Alertmanager (Apache-2.0; a scrape poll and a TSDB) (01 §2; 05 §3.1).
- Conflict 10 (the baseline, ≈265 vs ≈100–120 a minute) is not settled by arithmetic: the brief quotes no Layer 9 percentage over a counted baseline
  (01's "+0.06 %" is dropped). Layer 9's gate is that Layer 7's measured no-timer line (L7 §3.7) does not move with Layer 9 present and idle, and G7
  (`zero-weight.sh`) shows the same processes, listeners and idle RSS (05 §3.1).
- A hung (not dead) `111d` on a phone is detected by nothing today, since the cron log has no reader; removing the cron leaves that unchanged (§5 Q4).
- Amends: Layer 1's unit (`ExecStopPost=`) and Termux files (the inbox line), additive; one removal, the cron line (F13 item 2); L4 gains an `incident` SSE name.

### F3. Where Layer 9's state lives: the joint storage contract (joint with Layer 8; critic fork 12b, conflicts 9 and 11, gap G4; Layer 8 critic conflict 9)

- **A. One contract for both layers, below, worded identically in the Layer 8 brief.** Layer 9's records go into Layer 8's state service under `heal/`,
  encrypted at rest by the file Layer 8 opens for agent state; three small boot-time files stay outside it because a shell hook writes them or they must
  be read before the store opens; pruning is by capacity in Layer 8's budget; order is the store's own sequence; no own hash chain; one tag table.
  *(pick: `8-storage/research/04-local-store.md` §4 Fork 5 A and `8-storage/research/07-gap-1.md` §4 A; this layer's 07-gap-2 F5; 01 §3.2 for the record shape)*
- B. Layer 9 keeps its own files (01's `state/incidents.jsonl`, the `events.jsonl` FIFO pattern, `A/events.rs:17-21`: 1 MiB trimmed to half): no build-order
  dependency on Layer 8, but plaintext evidence excerpts on flash and a second budget outside Layer 8's per-class figures. Kept as the fallback (item 7).
- C. An own node-key hash chain for the audit (05 §4.1 item 5): rejected. Identity keys never sign routine traffic (fact 17), the chain's head is anchored
  nowhere outside the node, so it detects nothing a same-uid process could not rewrite, and a signed per-author chain is p2panda-core's job the day a
  shared feed exists (`8-storage/research/03-shared-log.md` §2 last row: writing one ourselves "would reinvent p2panda-core"; findings 8–9).

> **Joint storage contract (joint with Layer 8; the Layer 8 brief carries the same seven items, verbatim).**
> 1. **Home.** Layer 9's durable records (incidents, apply attempts, the loop ledger, kept-change snapshots, received fixes, the audit trail) live in
>    Layer 8's node111 state service under the namespace `heal/`, inside the `111d`-owned encrypted file Layer 8 opens for agent state (Layer 8 F1).
>    `111d` is their only writer; the agent reads them only through GET tools.
> 2. **Before the store opens.** Three small files stay plaintext under `state/` (0600, atomic writes, the probation-journal pattern), because a shell hook
>    writes them or they must be read before the store opens: the run marker, the witness inbox (one line per daemon exit: code, signal, boot id; no free
>    text) and the armed undo record of an apply in progress. `111d` reads them at start before serving, folds them into `heal/` and clears them; if the
>    store does not open, they stay and the Repairs page says so.
> 3. **Pruning.** By capacity only, as its own class in Layer 8's node budget: oldest closed incident first by the store's sequence; never an open
>    incident, an armed undo, an undecided proposal or a running task. A prune leaves one marker record (`pruned through seq N, K records`). No age, no
>    expiry, no wall clock.
> 4. **Order and integrity.** The store's monotonic sequence orders the records, with the boot id and monotonic time beside it; wall time is a display
>    claim. No own hash chain and no shared log in v1. A future shared fix-attestation feed must pass Layer 8's admission gate (a named consumer, one
>    writer per key, the never-list, a measured Pi delta), with p2panda-core as the adoption target.
> 5. **Signatures and tags.** Identity keys sign "announces, proofs, and control artifacts only — never routine traffic" (L2 §6.1). Layer 9's own rule
>    is stricter: it signs nothing that stays on the node (for Layer 9 the node key signs only its two outbound artefacts, `fix/v1` and `fix-vouch/v1`),
>    and no incident, proposal or audit record is signed. Every new node-key signature carries its own `111/<name>/v1` tag, registered in the one L2
>    table (§13.1 A4) in one amendment: Layer 7's `111/epoch/v1` and `111/broadcast/v1`, Layer 9's `111/fix/v1` and `111/fix-vouch/v1`, and Layer 8's
>    `111/catalogue/v1` and `111/export/v1`. Key-derivation inputs (Layer 7's `111/carrier-topic/v1`; Layer 8's `111/node-state/v1`, which derives the
>    key of the item 1 file) sit in the same table as key-derivation tags, never as signing tags.
> 6. **Nothing leaves.** Incidents, evidence and the audit never leave the node, except inside the user's own encrypted store export (Layer 8 F12),
>    which only this node's key opens. A shared fix carries only the `fix/v1` fields `111d` builds; a delegation carries only the `AGENT_OP` request fields.
> 7. **Fallback.** If Layer 8's agent-state file is not built when Layer 9 builds (or the owner takes Layer 8 F1 B), `heal/` is a `111d`-owned JSONL
>    ring under `state/heal/` (0600, 1 MiB, the `events.jsonl` pattern) under rules 2–6, moved into the store once it exists.

- Costs: on Layer 9's side none resident; disk bounded by the class share Layer 8 sets (the incident estimate: 600–1,000 B a record, so 1 MiB holds about
  1,000–1,700, 01 §3.2, unmeasured); the second SQLCipher connection and its page cache are Layer 8's cost (unmeasured, 8-storage 06 §5).
- Amends: L2 §13.1 A4 (additive rows); L3 `README.md:50` reads true once Layer 8's file exists (§4).

### F4. Diagnosis: the tiers, and where a model runs (critic fork 3; conflict 6; 01 fork 5; 05 fork E; G7)

- **A. Deterministic first, Layer 3's model second, the hub third; Layer 9 never loads a model by itself.** T0: the playbook's checks, evaluated by
  `111d` with no model and no agent (every tier that runs Layer 9). T1: Layer 3's model, only where Layer 3 already runs one (2B where the owner took the
  model offer, 4A, 4B), only when T0 matches several causes or none, as a background task, answering a pure classification constrained by a JSON-schema
  grammar to the playbook's cause ids (llama-server turns it into GBNF, 01 §3.3), fed the incident record and the cause list (≈0.5 KB, 07-gap-2 F4),
  never today's ~6,000-token context; any narrative is a separate call (G7: format constraints can hurt reasoning and help classification, Tam et al.,
  search snippet). T2: the home hub (F11), else "needs a hub". Containment outside the model: cited evidence ids must exist in the incident, quoted
  excerpts must be substrings of it, confidence is computed (share of checks satisfied; rule/model agreement) as two fields, the model never grades
  itself, the output is a proposal. Cost: T0 zero; T1 one model call per unresolved incident, about a minute for a 150-token verdict on a Pi 5 (fact 11,
  unverified); the reload after `--sleep-idle-seconds` is unmeasured (G6). *(pick: 01 §3.4 tiers; 05 F7; Layer 3's profile table settles conflict 6: 01
  wanted the model on every 2B, 05 only on 4A/4B, Layer 3 gives 2B the model as an offer, `3-intelligence/README.md:33`)*
- B. Load the model for repair on every 2B Pi: 3–4 GB that Layer 3's offer left to the owner; fails the Pi floor. C. Rules only: no unknown-cause path and
  no narrative. D. Model only: fails on 2A, 3 and slow Pis.
- Amends (additive, Layer 3): `diagnose` takes a structured `incident` object rendered in a labelled untrusted block, which also switches off the daemon
  health and journal context for that task (07-gap-2 F7); two GET tools join the fixed set, `incident.read` and a bounded child-log tail (D10's set grows,
  as D20 and D27 did).

### F5. Playbooks: home, grammar, evaluator (critic fork 4a; conflict 5; G3; 00 Q5; 01 fork 4; 04 fork C)

- **A. A Layer 0 `playbook` category with a closed grammar, evaluated by `111d`.** Folder `playbooks/`, filename `repair-<subject>-<reason>.md`, no new
  frontmatter field (`relations` carry `depends_on` the project and `runs_on` the devices), the machine part a fenced TOML block under `### Checks` inside
  `## Details`. Grammar v1 is a closed predicate enum: `service_state`, `service_reason`, `restarts_at_least {within_events ≤64}`, `incident_count_at_least`,
  `exit_is`, `health_is`, `file_exists` (a metadata call, never a read), `log_contains` (a literal ≤64 B in the last 100 lines or 8 KiB, no regex),
  `disk_free_below`, `unclean_exit`, `component_enabled`, `release_at_least`; combinators `all`/`any`/`not`, depth ≤3, ≤16 leaves, ≤2 KiB. A cause's action
  deserialises into the applier's own action type plus `advise` and `escalate`; the one non-literal is `"@subject"`; no templating, no command, no URL.
  `schema` is an integer, every table `deny_unknown_fields`, the predicate enum has no fallback variant: an unknown predicate, field, service id or reason
  code rejects the whole playbook ("needs release X"), never one check. Playbooks ship in the signed release (`share/kb/`), and the KB's rubric applies:
  `proven` means it healed a real incident, logged in `### Field log`. Cost: ~200 lines and a facts trait, 0 crates, about +10–30 KB (estimate); the lint
  uses stdlib `tomllib`, no new Python dependency. Prior art read: StackStorm rules (closed operators, but Jinja-templated action parameters), the Salt
  reactor (free templating), Nagios and Icinga handlers (state edges, command-line actions), Ansible handlers, Kubernetes probes, MAPE-K; ours closes the
  action side too. *(pick: 07-gap-3 §4 A)*
- B. `cel` 0.15.0 (MIT, licence read) behind a cargo feature on hubs: 11–23 crates and +0.7–3 MB (estimates), pre-1.0 churn, and a playbook valid on a hub
  is inert on a Pi. Revisit at the first playbook the enum cannot say. C. Playbooks as signed files outside the KB: loses the rubric. D. 00's
  `repair-*` recipes (breaks N03's deterministic split) or 01's `expect:` strings (a hand-written parser). `serde_json_path`, `jaq`, `rhai`: rejected (07-gap-3 §2).
- Conflict 5 settled: the home is a Layer 0 category (04 and 07-gap-3), not `skills/` (01; it stays deferred), not a recipe grammar (00), not a node
  overlay (02; F12); the evaluator is `111d`, not "111-agent code" (01), because T0 must run with no agent and no model.
- Amends locked L0 (additive): `playbook` in the category enum (F05), one N-rule (filename = the block's `[match]` = the closed vocabulary), the H3
  vocabulary under `## Details`, the TOML lint; recipes and N03 untouched.

### F6. The change engine: typed actions, the undo, after the keep (conflicts 1, 3; 00 fork 2; 02 forks A, B, C, E; 05 fork C; P4)

- **A. Generalise the Layer 6 applier now behind an `Action` trait, with a `repair` proposal kind carrying typed actions** (conflict 1: 02 waited for "a
  fifth action family"; settings, components, bridge actions and update rollback make it here). Per action: a pure `plan()` dry-run (diff, preflight, a
  `reach` class), an armed undo record written before the first effect (0600, atomic) and resolved at daemon start before serving ("armed at start =
  revert", the ESP-IDF `PENDING_VERIFY` rule: no clock), apply, a probe that asserts the capability (an interface listed and up, a helper re-attached;
  never "is it quiet?", the Kali lesson), keep or revert. The reachability class (`listen_lan`, a port, `hub_role`, a carrier, pairing): the human confirms
  from the same URL within 90 s (LuCI's window, search snippet), else revert; the timer exists only while pending; an action that would
  remove the last pairing or the loopback listener is refused. **After the keep** (conflict 3: 02 said a new proposal, 00 and 05 a post-keep
  probation, 05 five snapshots against 02's one `undo.json`): both records, no window. The armed undo lives only during the apply; the last
  five kept snapshots are retained by capacity; a new incident on a target whose last kept change still has a snapshot offers "Undo <change>" (linked by target, count-free, clock-free, so no re-probe poll,
  G6); Undo is a human act that runs the same loop. A ledger keyed by `sha256(action, target)` counts applies and reverts: after a revert the same action
  for the same cause is not re-proposed until the human re-accepts or the evidence changes; a target with an armed undo answers `409 busy`.
  `diagnose` and `propose` stay advisory (accepting means "read"); their actions arrive as a `repair` proposal. Cost: one small write per apply, at most
  five snapshot copies (`interfaces.toml`, `rns/config`, settings files; sizes unmeasured); no idle timer. *(pick: 00 §4 fork 2; 02 F1–F18, forks B1, C2,
  E1; 05 F1 and fork C)*
- B. Extend in place, the trait later (02 A1): a smaller diff now, a port later. C. An adopted engine (NETCONF, Nix generations, ostree, A/B partitions):
  nothing fits the floor (no spare partition on a Pi or a phone; reboots) (02 §2).
- The v1 vocabulary (closed): `service.restart`; the four device actions (D29 behaviour byte-for-byte); `settings.set` on named keys; `component.enable`
  and `disable` (each with the component's own consent and licence notice, D26 (5): a download only on that consent); `bridge.edge.pause`,
  `bridge.edge.set`, `bridge.order.set` (F12); `update.rollback` (§3); `advise` and `human_step` (nothing to run). Never typed: `firmware.flash`; still
  refused: `meshtastic.set`, `ble.*`; out of scope: identity, keys, the update channel, consent records.
- Validation by category (02 §3.4, fork C2): every node runs `plan()` and the action's own probe (a phone gets exactly that, said plainly in the UI);
  a Pi adds supervisor events as probe inputs; a hub adds, for pull requests only, F7.
- Amends: L6 §12, additive (the trait, the persisted undo, the reach class, the vocabulary) and one correction (the start-up reconciliation, fact 4); L4's
  contract (`Proposal.kind` gains `repair`, `fix`, `human_step`, `kb_draft`; the `applied` record joins `Proposal`, `proposals.rs:170-180`; `pnpm gen:types`).

### F7. Code fixes in v1, and what "validated" means for code (critic fork 5; conflict 7; gap G1)

- **A + B. Typed fixes everywhere; code fixes only as drafts on a hub that opted into the patch component.** opencode (MIT, v1.19.5 licence read) runs
  under a deny-all permission block (read, glob and grep allowed; bash, edit, webfetch, websearch, external_directory, task, skill, lsp and question
  denied; autoupdate off, sharing disabled), with `HOME` and the XDG dirs in the task directory, the five `OPENCODE_DISABLE_*` flags of 07-gap-1 F2, never
  `--auto`, against a read-only copy of the exact release's source (a hub-only, hash-pinned manifest asset, 1.7 MB gzipped, `prefetch = false`).
  `111-agent` checks that the unified diff parses and applies cleanly to that tree (`diffy` 0.5.2, MIT OR Apache-2.0, both licence files read) and files
  it as text; the proposal says **"drafted and applies cleanly, not built"**; upstream CI validates after the human opens the PR. Conflict 7 settled:
  README §9's "internally validates" means the action's own probe for typed changes; for code it means "applies cleanly" before the PR and CI after (03's
  reading), with 02's pre-PR hub sandbox as option C later. **The containment fixes are a Layer 3 correction the lead may commit before the Layer 9
  decision** (they change no Layer 9 behaviour and restore L3 `README.md:55`; the `patch` kind is exposed today, fact 8): the deny block, the `HOME` fix,
  and a namespace gate in which a prompt that asks for `curl 127.0.0.1:11180` must produce no connection. Costs: nothing resident; opencode 211–222 MB on
  disk (npm `unpackedSize`), its RSS while running unmeasured ("hundreds of MB to GBs in long sessions", issue snippets, unverified); **`diffy` is a
  build-time weight on every `111-agent`, Pi builds included (delta unmeasured; one `LICENSING.md` row and a notices regeneration): it needs the owner's
  yes** (AGENTS.md); without it the draft is filed unchecked and the owner's `git am` is the first check. opencode, as a pinned hub program, gets its own
  `LICENSING.md` row. *(pick: 07-gap-1 §4 B)*
- A alone: no code fixes in v1. Zero cost; README §9's "packages it as a pull request" waits.
- C. B plus sandboxed `cargo check`/`test --offline --frozen` of the two pure-Rust workspaces in bubblewrap 0.13.1 (LGPL-2.1 text read; "or later"
  unverified) or nsjail (Apache-2.0), a pinned Rust toolchain component (111–127 MB xz; MIT OR Apache-2.0) and a ~100-line namespace forwarder for the model
  port; build time and RAM unmeasured. Later, on hubs that opt in, after a measured cold `cargo check` on a Pi 5 and an N100 (§5 Q12).
- D. C plus a `node111` release build (84 MB of crates, a C toolchain, musl cross compilers, the built UI): not recommended.
- No git on the node or the hub: the diff comes from opencode and the human's own git opens the PR. Amends: L3 `README.md:55` holds again on hubs;
  `:86` G2 re-tested (§4); Layer 1's manifest gains the hub-only source asset (additive); opencode becomes a pinned, consented hub component (none exists
  today: no installer, no manifest, KB status `candidate`, 00 trap 6).

### F8. Pull requests: who sends, and the sign-off (critic forks 6, 10; 03 forks B, E; 05 fork D; README:20)

- **A. The human sends.** The hub renders `fix-<id>.mbox` (a `git am` mbox with git's fixed datestamp and `base-commit:` set to the release, so no wall
  clock) and `PR.md` (fix id, author node, incident id, the diagnosis as a quoted block, the verify log `111d` recorded, what was not verified, the tools and
  their versions, `Assisted-by:` trailers). The agent never adds `Signed-off-by:` (the kernel's current rule). The owner runs `git am` and `gh pr create`
  (MIT, on their own machine). A phone gets a prefilled GitHub issue URL for typed and kb fixes. One PR per fix id, never retried automatically. No token on
  any device. *(pick: 03 §4.2 B2 + B3; 05 fork D)*
- B. An opt-in hub token (v1.1): a fine-grained single-repository token on a hub, off by default, its consent naming the repository and a daily cap; never
  reachable by opencode (the Invariant Labs leak through a public issue, search snippet). Whether a fine-grained token can open a fork-based PR is unverified.
- C. A project GitHub App, or an intake Action turning issues into PRs: rejected (`pull_request_target` is the documented "pwn request"; untrusted text would
  drive a privileged workflow).
- README:20 ("the agent … sends updates and PRs") reads as the progressive goal; in v1 the agent prepares and the human sends (§4). Licence: D22's inbound
  = outbound stands; the grant comes from the human's review and sign-off (03 finding 9; not legal advice). DCO: §5 Q2. Cost: none
  resident; two text files per fix on the hub's disk. Amends: no locked spec; `.github/PULL_REQUEST_TEMPLATE.md` gains a sign-off line (§5 Q2).

### F9. Signing: who signs what leaves the node (critic fork 9; conflict 11; 00 Q2)

- **A. The root key through `111d` (the identity holder) with two new registered tags; the agent holds zero key material.** `111d` builds and signs the
  artefacts that leave the node from records it holds: the fix artefact under `111/fix/v1`, and any statement one node makes about another's fix id
  (`vouched`, `reproduced`, `accepted`, `reverted`, a kind byte inside the signed bytes) under `111/fix-vouch/v1`. It never signs bytes the agent hands it
  (no signing oracle). Proposals, incidents and audit records are not signed: L2 §6.1 keeps identity keys to "control artifacts only — never routine
  traffic" (fact 17), and Layer 9's own, stricter rule signs nothing that stays on the node (joint contract item 5). `AGENT_OP` payloads need no new tag:
  `111/msg/v1` covers them (07-gap-2 F8). 00's `111/proposal/v1` and `111/patch/v1` are dropped: a proposal never leaves the node, and `PATCH` is the frame
  that carries a `fix/v1`. Cost: two rows in the L2 table. *(pick: 03 §4.1; 02 fork F2; conflict 11 settled for 03's names, in one L2 table with Layer 7's)*
- B. Activate the L2 delegation cert (an agent key, scope bits, expiry in mesh epochs): Layer 7 evaluates the epoch for no consumer in v1, and peers that
  are not upgraded reject agent-signed traffic. v1.1 or later, with the epoch's first consumer.
- C. Unsigned fixes: impossible; `PATCH` is SIGNED-mandatory (`5-messaging/README.md:139-140`).
- Amends: L2 §13.1 A4, additive; L3 `README.md:49` ("the agent signs … with the node key") reads "`111d` signs on its behalf" (§4).

### F10. Sharing proven fixes between nodes (critic fork 7; conflicts 8 and 9 (exchange); 03 forks C, D; 05 forks F, F8)

- **A. v1: a sealed `PATCH` carrying one `fix/v1`, sent by a human to the home hub or a contact the user pinned, filed there as a `fix` proposal.** The
  artefact (03 §4.1): UTF-8 JSON signed as exact bytes; `fix_id` = BLAKE3 of the body; `author_seq` a per-author counter (replay = not increasing); the
  mesh epoch if known, no wall clock; `class` typed, code or kb; `applies_to` {category, layers run, release calver, component versions}; the incident
  fingerprint and a short diagnosis; the verify log `111d` recorded and a `not_verified` list; at most 4 KiB, a code attachment at most 64 KiB. Size: one
  `tcp-lan` frame; a 233 B bearer only when it fits 16 fragments; never the 160 B floor; across islands it rides Layer 7's custody hand-over like any sealed
  item (L7 §5), and carrier bytes count in Layer 7's metered cap (L7 §12). Receiver, all before a human sees a button: verify the signature
  (`RECEIPT_STRONG` returns the hash); author = the home hub or a pinned contact, else parked "unknown author" with no action; `code` and `kb` classes show a
  read-only diff and offer only "submit upstream"; `111d` checks `applies_to` against the node's live scope; typed actions are re-validated against this
  node's own vocabulary and a scope list that refuses anything touching the gateway, a carrier, identity, keys, the update channel or consent records;
  then a proposal. Accepting runs F6's loop and sends back a `fix-vouch` statement. Defaults: receive "from my home hub", send **off** (a fix says what
  failed here; its consent shows the exact bytes). **Conflict 8 settled: the receiving agent never reads or summarises foreign fix text**; `111d` renders
  the card from typed fields and the prose preformatted (03's "the receiving agent's job is to summarise" is dropped). Maturity: `local` → `reproduced` →
  `accepted_by_n` (n owners accepted it and none reverted it within the next K incidents of its fingerprint: a count, not a period, G6) →
  `upstream_merged` → `in_release`. Cost: zero processes, listeners or timers; disk up to 256 fixes ≈1 MiB (03 §4.6, estimate). *(pick: 03 §4.3 C1 + D1)*
- B. No sharing in v1 (wait for Tier 2): no Layer 5 change; README §9's "share proven fixes" waits.
- C. A hub "fix shelf" members pull by `applies_to`: v1.1, through Layer 8's origin admission rule with a hub-owner-signed fix index as a pin source
  (fact 23), joint with Layer 8. D. Open gossip or a Radicle-style patch graph: rejected (git and a daemon, floods metered links, no human per receiver).
- Trust: D1 the home hub plus pinned contacts (pick); D2 plus authors a paired hub vouched for (v1.1); D3 anyone (rejected).
- Conflict 9, exchange half: 05's "code pulled `111 update --from`-style" does not exist; the hub origin serves release files only (fact 7; 00 trap 12).
- Amends: Layer 5, additive (F13 items 11–14).

### F11. Delegation for Cat 2A and 3 over the fabric (gap G2; README:118; the `AGENT_OP` hand-off, `5-messaging/README.md:832`)

- **B. `AGENT_OP` in v1, one closed op pair, home hub only.** Ops `diagnose` (phone → hub), `diagnosis` (hub → phone) and `refuse {reason}`;
  deterministic CBOR with a `schema` byte first, every field length-capped. `111d` builds the request (≈450–600 B, arithmetic) from the incident record:
  fingerprint, release, category, the checks that ran and their results, the causes ruled out, at most three excerpts of ≤120 B, nothing from a bridge
  adapter or presence log. The reply (≈400–600 B) carries a cause id from the closed playbook set, the two confidence fields, at most three typed actions
  and ≤256 B of narrative. Size-gated: `tcp-lan` and 233 B bearers; on the 160 B floor the request waits as "needs a longer link". Queued in the phone's
  outbox until a path exists; no poll. The hub accepts only from an `enrolled` member on an attributed connection, the phone only from its `home_hub` for an
  open request; one open request per member; the hub switch `heal.serve_members` is off by default. **The member's egress has its own consent, "Ask my
  home hub to diagnose" (off / on), default off**, since a request says what failed on this node (05 §4.2's reason for "send fixes: off"). Turning it on
  is a widening whose `409 consent_required` lists the exact request fields (the L6/L7 shape); the card's "needs a hub" badge links to it. Under it `111d`
  sends the request automatically when an incident opens with no T0 cause match (07-gap-2 F5's trigger); off, nothing is composed or sent (§5 Q23). The
  hub's agent diagnoses in a labelled untrusted block with its own health and journal switched off. The reply becomes a proposal the phone's human accepts,
  never auto-applied (even under F1 B), its actions re-validated against the phone's vocabulary. Costs: nothing resident on either end; one switch on each
  end, both off by default (`heal.serve_members`, "Ask my home hub to diagnose"); the hub serialises one task at a time over loopback and loads the model
  on demand; per-request RSS and binary deltas unmeasured. *(pick: 07-gap-2 §4; the member-side consent is this brief's, §5 Q23)*
- A. No fabric delegation in v1: T0 on the phone and a "needs a hub" badge; zero amendments.
- C. Finish Layer 3's LAN offload for Layer 9: a model port on the LAN (plain HTTP, no auth as built), a ~24 KB prompt per task, dead off the LAN. Rejected.
- D. A Layer 7 carrier ride: not a candidate; a carrier moves frames between paired hubs, never to a member, and a put addressed to a hub itself is refused
  (L7 §5–§6; 07-gap-2 F9).
- `patch` ops stay out of the op set until F7's containment is built and tested. Hub-to-hub delegation is v1.1 with a Layer 7 "hub as destination" amendment.
- **The APK refresh** (`1-bootstrap/README.md:160`) is local, not delegated: a `human_step` proposal "A newer Termux add-on is pinned", composed by `111d`
  when the signed manifest names an `apk.*` sha256 other than the one this phone last installed (recorded in a new, additive `receipt.apk` map written by
  the phone installer), with the steps "run `111 install --phone` again from your computer"; it never implies an automatic install (07-gap-2 F10).
- Amends: Layer 5 (R1–R3, F13); Layer 3 (the `incident` input, F4); Layer 7, additive to the locked §7.2 table: role bit 6 `ROLE_HEAL` (bit 4 `ROLE_OFFLOAD`
  means model inference, and a model-less hub can still serve playbooks); Layer 1 (`receipt.apk`).

### F12. Evolution: the agent, the KB, the bridging logic (critic forks 4b, 11; conflict 4; 04 forks A, B, D, E)

- **A. Release-only agent changes with the data seam built now; export-only KB drafts; three typed bridge actions.** Prompts and playbooks move out of Rust
  constants into signed release files (`share/agent/`, integrity-checked like `share/kb/`). A prompt, playbook or model-pin change is a release that passed
  a frozen must-not-break suite in CI (a stub model server replaying recorded outputs; schema, refusal and injection cases; fixtures from the G1–G7
  namespaces; zero regressions, paired; the score is evidence, never the gate: about 50 cases give ±13 points at 70 %, 04 §1). **No node-local overlay in
  v1** (conflict 4, settled for 04 against 02: a prompt has no probe that proves it safe, so 02's "auto-eligible under a grant" is withdrawn; an overlay, when
  built after a corpus and a suite exist, is a human-accepted change with a revert). The KB: a `kb_draft` proposal (target path, body, a `not_verified`
  list, incident refs); accepting it saves a file for the human's PR; the node's KB is untouched; a draft enters as `candidate` with a `### Smoke test`
  generated from the incident, and the agent never edits `status`. Bridging: `bridge.edge.pause`, `bridge.edge.set {oversize, honour_ok_to_mqtt}` and
  `bridge.order.set` run by F6's loop, plus playbooks; the human accepts each one, so "it cannot do so" (`6-transport/README.md:458-459`) stays literally
  true; no filter language (README §5: no usage policies). Cost: zero resident; the loader's binary delta and the bytes per new kind are measured at build,
  and `111-agent`'s 5.3 MB idle RSS must not move. *(pick: 04 §4 A1, B1, D1, E1 + E2)*
- B. The node overlay (04 A2) in v1: versioned `overlay/vN/` with current and previous, probation, a hub eval report; no corpus or suite exists to gate it.
- C. Auto-apply of signed data from a hub (04 A3): rejected (data with no probe; the DGM and METR evidence, fact 10).
- D. A resident canary on the node, or on-node fine-tuning (16 GB of FP32 weights alone for a 4B model; already refused by D10): rejected.
- Later, human-started hub tasks: replay of the owner's own incident corpus (an opt-in sealed export), GEPA (MIT) proposal runs, Inspect AI (MIT) as an
  optional runner; never a node loop (50 cases × 3 × 400 tokens = 60,000 tokens, 3.3 h at an assumed 5 tok/s on a Pi).
- Amends: L6 §8 and §12 (three actions, additive); Layer 3 (prompts become release files: code moves, the contract does not).

### F13. Amending locked layers (permission, not design)

- **A. Layer 9 may amend locked layers additively, exactly these items, each owner-visible (one removal and three corrections, marked); with Layer 9's
  switches off and no `heal` traffic, wire bytes and contracts are unchanged.** L0: (1) the `playbook` category, folder, N-rule, H3 vocabulary and TOML lint
  (F5). L1: (2) the `ExecStopPost=` line and the Termux inbox line (F2), and the cron entry removed (a removal the placeholder text invited, §4 item 2);
  (3) the hub-only source asset in the manifest (F7); (4) `receipt.apk` (F11); (5) the probation wording (§4; a correction). L2: (6) `111/fix/v1` and `111/fix-vouch/v1`
  in the A4 table (F9; joint). L3: (7) the `incident` input to `diagnose`, `incident.read` and a child-log tail tool (F4); (8) the opencode deny block and
  `HOME` fix (a correction; it may land before this decision, F7); (9) prompts as release files (F12). L4: (10) `Proposal.kind` gains `repair`, `fix`,
  `human_step`, `kb_draft`; `applied` in `Proposal`; an `incident` SSE event; `GET/PATCH /api/v1/repairs/settings` and the incident routes; the Repairs page
  `/settings/l9` as a lazy chunk ≤64 KB (D21). L5: (11) `Fabric::send_op(to, stream, kind, payload, receipt_strong)` for `AgentOp` and `Patch`, with the
  Tier 0/1 size gate and the same commit-before-seal rule as `send` (07-gap-2 R1, widened to 0x13); (12) `STREAM_AGENT = 1` (R2); (13) a consumer hook for
  kinds 0x10 and 0x13 instead of a chat row (R3 widened; the textless-row defect ends for those two kinds); (14) beside the Tier table's "every bulk class"
  (`5-messaging/README.md:239`): "a `PATCH` that fits one `tcp-lan` frame or 16 Tier 1 fragments may travel without Tier 2". L6: (15) the applier's trait,
  persisted undo, reach class and the vocabulary of F6 and F12 (the four device actions byte-for-byte), and the start-up reconciliation (a correction of
  fact 4: "armed at start = revert" replaces §12's "a crash mid-apply leaves a visibly half-applied proposal", `6-transport/README.md:628-629`; "never a
  second apply" stands). L7: (16) role bit 6 `ROLE_HEAL` (F11). Wire-visible additions: one op schema inside `AGENT_OP`, the `fix/v1` body inside `PATCH`,
  one role bit; old nodes drop the kinds as today. *(pick)*
- B. Layer 9 stays outside the locked layers: the observer, engine and exchange become a sidecar that duplicates the supervisor, the applier and the
  fabric. Rejected on "never reinvent". The minimal reading is A without items 11–14 and 16: F10 B and F11 A (no Layer 5 or Layer 7 change; T0 only
  on phones).

### F14. Sequencing and the first Layer 9 release (critic G8)

- **A. v1, in this order:** the applier's start-up reconciliation and atomic proposal and task writes (the deny block and `HOME` fix land earlier, F7) ·
  F2 observer, incident record and witnesses · F3 per the joint contract · F5 grammar v1 with playbooks for the services' reason codes, and F4 T0/T1 · F6
  engine and vocabulary · F1 A (the grant format pinned, off) · F9 tags · F11 the op pair · F10 typed fixes to the home hub and pinned contacts · F8 mbox
  and `PR.md` · F7 B on opted-in hubs · F12 the data seam, `kb_draft`, the bridge actions, the CI suite · the Repairs page · the APK `human_step`. **v1.1:**
  F1 B if the owner wants it · F7 C · the fix shelf (with Layer 8) · vouched authors · a hub token · hub-to-hub delegation · the node overlay · hub eval
  replay · the opt-in probe · delegation certs · Tier 2 fixes and LoRa fix cards. Layer 9 builds after Layer 8's state service; if it must build first,
  the joint contract's item 7 applies. *(pick)*
- B. The observer and the Repairs page only (T0, propose): the smallest cut; README §9's "share" and "pull request" wait a cycle.
- C. Everything at once, F1 B and F7 C included: unmeasured hub build costs and an open D26 reading in one release.
- Every gate stays netns- or stub-verified; the bench items are in §6.

## 3. Detail picks (spec-reviewed by the panel, not polled)

- **The incident record** (01 §3.2): fingerprint = (layer, subject, `reason_code`); `count` makes a crash loop one incident; `opened`/`last`/`closed` carry
  the store's `seq`, monotonic ms, boot id, the daemon tick, `epoch | null` and a display-only `wall_claim`; evidence = pointers plus excerpts ≤512 B from
  typed sources (the service record, health JSON, counters, our own child log tails), never message bodies; `attempts` and `outcome`. Field names are
  borrowed from the Kubernetes Event, the journal's `_BOOT_ID` + monotonic pair and OpenTelemetry's dotted names; no SDK.
- **Incident keys**: `111d`'s `/health` is two-state (`N/health.rs:24-41`) and the tri-state is the agent's own (00 trap 2), so `health_is` reads both;
  service rows are keyed on (service id, `reason_code`); optional components are not service rows (`Components::status`); `Degraded` is the
  "needs repair" signal; at start, the update tombstone (`pending:false`, `health_confirmed:false`) becomes "update reverted" and the guard report "torn
  payload" (00 §3.1 (a)).
- **The Repairs page** (05 §4.1): one verdict line, Needs you, a Timeline of incidents (state chip, the actor of the last step, expandable evidence,
  attempts, the verify log, outcome), Changes with Undo while a snapshot is kept ("a restart cannot be undone" otherwise), the Audit (advanced), the
  consent block. Event-fed: no UI timer and no ticking countdown. Eleven honest states, `refused` and `unable-to-diagnose` among them; three severities
  and a dismissal that survives restarts and re-opens on a changed cause (Home Assistant's issue registry, Apache-2.0, a reference). The lazy bucket had
  36.1 KB free before Layer 7's pages (L7 brief fact 18); whether Repairs fits after Layers 7 and 8 is unmeasured (a D21 question at build).
- **Consent and settings**: the autonomy ladder; receive fixes (off / **home hub** / paired hubs / pinned contacts); send fixes (**off** / home hub); ask
  my home hub to diagnose (**off** / on, members, F11); serve members (**off**, hubs); open PRs from this hub (**off**); notify phones (**transitions
  only**, content-free: "111: your node needs you"). `GET/PATCH /api/v1/repairs/settings` answers `409 consent_required` for any widening (the L6/L7
  shape, listing the exact fields or bytes that would leave), writes are loopback-only, a switch shows the node's answer, never an optimistic one.
- **The phone** (05 F6): it keeps the observer, the incident record, the typed applier, the proposal UI and the decision; it delegates diagnosis to its
  hub under its own consent (F11). It cannot check its own phantom-process hardening (that needs `adb`); it detects SIGKILL exits in the `finish` hook and
  files a `human_step` "re-run `111 install --phone`". An in-process observer spends none of Android's 32 phantom-process slots.
- **`update.rollback`** asks the root helper through `revert-request.json`, which it serves today for a READY-but-503 release (`1-bootstrap/README.md:158`);
  whether it honours one outside probation is a Layer 1 read at build (unverified); if not, the action is a `human_step`.
- **Honest words**: "probe passed" names the probe; "drafted and applies cleanly, not built"; "validated" is never self-assigned and always names its
  validator (METR's finding that models edit their scorers, snippet).
- **Untrusted text**: anything from another node, a bridge or an RF origin is data, rendered preformatted, never an instruction to the agent; the agent
  never reads its own drafts back as KB truth (MINJA, AgentPoison, MemoryGraft: persistent memory is an injection target, snippets).
- **Privacy**: incidents, evidence and diagnoses stay in `/var/lib/111` (save the owner's encrypted store export, joint contract item 6). Each egress names
  its trigger: a sealed `PATCH` a human sends and the `fix-vouch` reply to a fix the human accepts (F10); a `diagnose` `AGENT_OP` to the home hub at incident
  open while "Ask my home hub to diagnose" is on (F11), each built by `111d` from typed fields; a PR or issue only by the human's hands (F8). No telemetry (F2 C).
- **T1 discipline**: `111d` submits one model task at a time (the agent does not serialise concurrent turns, `3-intelligence/TASKS.md:72`); a reply that
  cites unknown evidence or quotes text not in the incident is discarded, with no retry loop.
- **Termux** (01 §3.6): netlink is blocked for apps on Android 11+ (snippet), so hotplug keeps its 5 s fallback and Layer 9 adds nothing; battery is
  pull-only through Termux:API (on demand: the page open, before a heavy task); no inotify listener is needed.
- **Linux PSI triggers** (`/proc/pressure/*`): an event source for memory and I/O stress with no timer, where `CONFIG_PSI` exists (unverified on Pi OS
  kernels); a later opt-in, not v1.
- **The gate for `9-evolution/TASKS.md`**: G7 extended so processes, listeners and idle RSS equal the baseline with Layer 9 present and idle, plus Layer 7's
  no-timer line; binary deltas of the observer, the evaluator, the engine and the op arm recorded (AGENTS.md "measure and record").
- **Rate shapes that are capacity, not policy**: one open incident per fingerprint, one open delegation per member, a count-bounded queue of incoming
  fixes per author with the oldest dropped and counted (the fabric's quota pattern); none is a usage limit on the owner (§5 Q1).

## 4. Plan corrections this brief records (the lead applies them after the decision)

1. `1-bootstrap/README.md:158`: "sustained across the 60s probation window" → 60 s of sustained green inside a 600 s deadline (`probation.rs:57`,
   `N/mod.rs:133-176`). Beside it: the probation monitor ticks every 5 s even with no probation pending (`N/mod.rs:136`; 01 Q6), a Layer 1 timer for the
   trim list; the deadline is wall-clock epoch seconds (00 trap 10), a timer, not crypto or ordering.
2. `1-bootstrap/README.md:131`: the cronie `/health` "placeholder seam until Layer 9" writes a log nothing reads (00 trap 9); F2 replaces it.
   `:160`: the daemon never reads the APK pins and the receipt records none (07-gap-2 F10); `receipt.apk` is the addition (F11).
3. `3-intelligence/README.md:51`: "restarts a dead child ≤10 s" holds for the first four exits; the fifth in 60 s is `Degraded` until a restart request (fact 2).
4. `3-intelligence/README.md:55` ("no host shell for the agent") does not hold on hubs today (fact 8); `:19`/`:35-36` call opencode adopted on hubs, but
   nothing installs it and its KB entry is `candidate` (00 trap 6); the code comment pins 1.18.18 behaviour while 1.19.5 shipped on 2026-10-28 (07-gap-1 §2).
5. `3-intelligence/README.md:86` G2 says opencode 1.18.18 "fetches its provider SDK on first use"; 07-gap-1 F4 read `@ai-sdk/openai-compatible` as a bundled
   static import in 1.19.5 (and line 117 of 1.18.18): the observed air-gap hang needs a re-test with the `OPENCODE_DISABLE_*` flags (which call hung is unverified).
6. `3-intelligence/README.md:48,110` and Layer 7's text repeating them (L7 §7, §13.3, §19): offload "on the LAN binding + pairing auth" is not built
   on the hub side, and G3 conflates the agent's `--lan` with the model port (07-gap-2 F2); a Layer 3/7 item, outside Layer 9 (§5 Q14).
7. `3-intelligence/README.md:49`: "the agent signs its outbound artifacts … with the node key" → `111d`, the identity holder, signs (F9; `2-identity/README.md:224`;
   `N/intelligence.rs:10-12`). `:84`: "proposal signing is deferred to Layer 9" → proposals are never signed; fixes are (F9).
8. `3-intelligence/README.md:50` ("in the encrypted local store"): not true as built (agent state is plaintext files, `8-storage/research/04-local-store.md`
   finding 1); true once the joint contract and Layer 8's file land.
9. `A/store.rs:120-146`: proposals and tasks are written non-atomically despite the module comment (`:2-3`); temp + rename + fsync (also Layer 8 07-gap-1 finding 6).
10. `4-interface/ui/src/lib/layers.ts:75` ("tests fixes in a sandbox") and `os/README.md:67` (§3: "test candidate fixes in a safe internal sandbox"): no
    sandbox exists on any node; the v1 words are F7's. README §9 "internally validates" and README:20 "sends updates and PRs" read as F7 and F8 say; the
    README stays user-facing and is reworded only if the owner wants (as with Layer 7's heartbeat sentence).
11. `4-interface/api/openapi.yaml:746-758`: the `applied` record is on disk but not in `Proposal` (`proposals.rs:170-180`); "reverted" is invisible in the UI.
12. `5-messaging/README.md:239`: the Tier table sends every bulk class by Tier 2, while 03 assumed a `PATCH` in one TCP frame (F13 item 14);
    inbound `AGENT_OP`, `KB`, `UPDATE`, `PATCH` land as textless chat rows (fact 14), ended for 0x10 and 0x13 by F13 item 13 and joint with Layer 8's
    question on the rest.
13. `6-transport/README.md:458-459` stays true under F12; the three bridge actions are an additive §12 vocabulary amendment.
14. `0-knowledge/README.md:32`: recipes stay install-only; playbooks are a new category (F5); `:143`'s deferred `skills/` stays deferred.
15. `plan/cross-layer/parked.md` P4: picked up by Layer 9 (F6); its restart clause is read per F1 (A: not built; B: built inside a grant).
16. The timer baseline (fact 19): 00 found three timers the Layer 7 count missed (the 2 s `status` tick, the 2 s agent-relay `absent_poll` on nodes without a
    running agent, the 30 s LAN refresh); Layer 7's `TASKS.md` baseline row should list them before its first G7 run.
17. `2-identity/README.md` §13.1 A4 gains Layer 7's, Layer 8's and Layer 9's tags in one amendment (joint contract item 5).
18. `plan/layers/9-evolution/README.md` "Files": `07-gap-N` reads "cap 2"; three gap-fills exist (one in run 1, two in run 2, `coordinates.md`).
19. Lens premises corrected by the scouts: Home Assistant core is Apache-2.0, not GPL (05 §1); P4 says restarts "may skip approval", superseded in tone by
    D26 (2) and narrowed by the critic's reading (06 §2.2).

## 5. Open questions for the owner (beyond the polled forks; the lead's default in bold)

1. README:79 says "no rate limits". **Default: capacity bounds on other nodes' input (one open delegation per member, a count-bounded incoming-fix queue
   per author) are capacity, not policy; every cap on the owner's own actions (PRs a day, an auto-apply count if F1 B) is a default the owner can set to
   unlimited.** (critic fork 8; 05 Q7; 07-gap-2 Q8)
2. A DCO sign-off on contributions, and a CI check? **Default: the PR template gains a DCO 1.1 sign-off line the human fills; no CI check in v1; D22's
   inbound = outbound unchanged.** (critic fork 10; 03 Q1)
3. Repairs in simple mode? **Default: the Needs-you card in simple mode, the full page in advanced.** (05 Q1)
4. A hung (not dead) `111d` on a phone: **default: no watchdog in v1 (as today); a Termux hang watchdog is a Layer 1 question.** (F2)
5. The reachability confirm: **default 90 s monotonic, from the same URL; on Termux, where loopback is not trusted, the paired phone browser confirms.** (02 Q2)
6. Refuse a proposal whose probe cannot assert a capability? **Default: yes, unless removing the capability is the proposal's stated purpose and the human
   accepted that text.** (02 Q5 and its finding F14)
7. Undo depth: **default the last 5 kept snapshots, by capacity.** (05 Q9) · RF-affecting actions never auto (if F1 B): **default yes, overridable.** (05 Q3)
8. `incident.read` and a child-log tail tool (D10's fixed set grows): **default yes.** (01 Q5) · The 64-incident ring and the record on 2A, 2B, 3, 4A, 4B
   (not Cat 1): **default yes.** (01 Q2) · A visible heartbeat of Layer 9 itself: **default no.** (01 Q1)
9. Predicates in grammar v1: **default the twelve; the lead may cut `release_at_least` and `component_enabled`.** Fixture home: **default beside the Rust
   loader's tests, not in the payload.** (07-gap-3 Q4–Q5)
10. Loopback-is-human on a node whose agent spawns children: **default kept for v1; the deny block and its namespace gate close opencode's path; revisited
    with F7 C.** (07-gap-1 Q3)
11. The hub-only source asset (≈1.7 MB) in the channel manifest: **default yes, `prefetch = false`, fetched when the patch component is enabled with
    consent.** (07-gap-1 Q2)
12. A Rust toolchain component for F7 C: **default 4B hubs only, after a measured cold `cargo check` of `3-intelligence` (time, peak RSS) on a Pi 5 and an
    N100.** (07-gap-1 Q4–Q5)
13. Excerpts from bridge-adapter and presence logs in a delegation: **default excluded, counters and reason codes only.** (07-gap-2 Q3) · `ROLE_HEAL` as a
    new bit 6 or a widened bit 4: **default bit 6, an additive Layer 7 lock amendment.** (07-gap-2 Q7) · Hub-to-hub delegation: **default v1.1.** (Q6)
14. Layer 3's chat offload (no hub listener or auth as built): **default outside Layer 9; recorded as a Layer 3/7 correction; chat offload stays
    documented-not-built until its owning layer fixes it, or later rides the same op pair.** (07-gap-2 Q4)
15. APK: **default the installer records `receipt.apk` (Layer 1, additive); the hub serving APK blobs waits for Layer 8's origin admission rule.** Which
    folder Termux can write that the Android installer reads is unverified. (07-gap-2 Q5)
16. Fixes from hub-vouched strangers: **default no in v1 (home hub and pinned contacts only).** (03 Q3) · One-tap accept of a typed fix from a contact:
    **default no, always the full review screen.** (03 Q4) · Send fixes: **default off.** (05 Q4)
17. A GitHub token on a hub, ever: **default v1.1, opt-in, fine-grained, one repository, never readable by opencode.** (03 Q2; 05 Q5)
18. Daily caps: **default no "Submit upstream" cap (one PR per fix id, a human click each); the incoming-fix queue holds 16 per author (the lead's
    number, to confirm), oldest dropped and counted; 03's hourly cap is dropped as a clock-based rate.** (03 Q5)
19. KB drafts upstream: **default the same mbox and `PR.md` route as code, no in-product send.** (03 Q6; 04 Q4)
20. The incident corpus without telemetry: **default project-written fixtures only in v1; the owner's opt-in sealed export to a hub later; a replay script in
    `check.sh` rather than Inspect AI.** (04 Q3) · Model-pin swaps: **default always a project release with an eval report; no side-loaded adapters.** (04 Q6)
21. Bench runs on the owner's machine: **default `llama-bench -p 512 -n 128` for the 4B Q4_K_M model on a Pi 4 and a Pi 5 with RSS (05 Q8), and the model
    reload time after `--sleep-idle-seconds` (01 Q7), before T1's numbers enter the spec.**
22. Probation numbers: **default keep both (60 s sustained inside 600 s) and fix the README text.** (02 Q4)
23. A member's delegation consent (F11): **default "Ask my home hub to diagnose" off; on (its `409` lists the request fields), `111d` asks at incident open
    with no T0 cause match.** The alternative: a human "Ask my hub" tap on each incident card. (07-gap-2 F5; 05 §4.2)

## 6. Critic and gap-fill

**Critic (`research/06-critic.md`): "enough for an options brief on the core"** (event-driven observation in `111d`, incidents, deterministic playbooks, the
typed apply-verify-undo engine, consent and the Repairs page), with one **blocking** gap: the hub code-fix path, unsafe as built; one missed hand-off
(`AGENT_OP`); twelve conflicts left for the brief. The critic settled four things itself: opencode runs with every tool allowed, bash included (upstream
`tools.mdx:8`, `permissions.mdx:174-176`); P4 against D26 is narrower than reported (D26 (2) governs the agent's bearer); git (GPL-2.0-only) and llama.cpp
(MIT) were read; the Pi floor includes `111-agent`. Gaps and how each was closed: **G1** (blocking, the hub code-fix pipeline) → `07-gap-1`, F7 (draft-only,
containment now, the sandbox later); **G2** (major, `AGENT_OP` for 2A/3) → `07-gap-2`, F11 and the APK `human_step`; **G3** (major, adopt vs build for rules) →
`07-gap-3`, F5; **G4** (major, the joint storage contract) → F3, worded for both briefs; **G5** (minor, the baseline measured) → left to Layer 7's gate at
build, and the brief quotes no percentage; **G6** (minor, unquantified items) → the post-keep window is event-linked with no window (F6), the debounce is one
incident per fingerprint (F2), "accepted_by_n for a stated period" is a count (F10), the model reload stays a bench item (§5 Q21); **G7** (minor, constrained
decoding) → T1 is a pure classification with narration apart (F4); **G8** (minor, no v1 cut across lenses) → F13 and F14. The twelve conflicts are settled
by name in: F6 (1, 3), F1 (2), F12 (4), F5 (5), F4 (6), F7 (7), F10 (8, and 9's exchange half), F3 (9's storage half, and 11 with F9), F2 (10, 12).

**Gap-fill (three scouts):** `07-gap-1` (run 1) showed headless opencode fails closed on `ask` but allows bash and web fetch by default, wrote the deny
block from upstream syntax, found the provider package bundled (no runtime npm install), measured the source asset and the crate volume of all four lock
graphs, and picked draft-only code fixes with "applies cleanly" as the honest word → F7, facts 8–9, §4 items 4–5. `07-gap-2` (run 2) found `AGENT_OP` to be a
byte with no schema, sender or consumer and Layer 3's LAN offload unbuilt on the hub side, and picked a narrow home-hub-only, size-gated op pair with no
Layer 7 carrier ride, three additive Layer 5 changes and the APK refresh as a `human_step` → F11, F13 items 11–13, facts 14–15 and 22. `07-gap-3` (run 2)
weighed `cel`, `serde_json_path`, `jaq`, `rhai` and a closed enum against StackStorm, Salt, Nagios/Icinga, Ansible, Kubernetes and MAPE-K, and picked a closed
predicate enum (about twelve predicates, zero new dependency) in a fenced TOML block of a Layer 0 `playbook` entry with no new frontmatter field, the
action side reusing the applier's types → F5, facts 12–13. Tensions resolved here: 03 assumed a `PATCH` fits one TCP frame, locked Layer 5 sends every bulk
class by Tier 2 → F13 item 14; 05 wanted code "pulled `111 update --from`-style", the origin serves only release files → F10; 07-gap-2 placed its pending
table in Layer 8's `heal/journal` "if Layer 8 Fork 5 A lands, else the incident file" → the joint contract with its fallback; 07-gap-2 asked Layer 7's owner
for bit 6 while Layer 7 was a draft, and Layer 7 locked today → an additive lock amendment; 07-gap-1 found the provider bundled where the Layer 3 G2 note
saw a download → a re-test (§4 item 5).

**Unverified claims (the lead verifies on the owner's machine; merged from the critic's §7 and the gap-fills' lists):**
- *Sizes:* the incident record (600–1,000 B) and the ring it implies; the `AGENT_OP` request and reply (450–600 B); today's ~6,000-token diagnose context; the
  fix body (≤4 KiB) and 256 fixes ≈1 MiB; the Rust toolchain unpacked, the vendored crate trees and the C toolchain; the Repairs chunk and the lazy-bucket
  headroom after Layers 7 and 8; opencode's on-disk size is npm metadata, not a measured install.
- *Binaries:* the closed-enum evaluator (+10–30 KB), `cel` (+0.7–3 MB), `serde_json_path` (+0.1–0.3 MB) are estimates from crate metadata, unbuilt; the `diffy`
  delta; the observer, engine, data loader and op arm.
- *Measurements:* the wakeup baseline (≈265 vs ≈100–120 vs ≈205/≈114), by Layer 7's §3.7 gate; the 4B model's generation and prompt speed, RSS and reload on a
  Pi 4 and a Pi 5 (2.3–5 tok/s are snippets; the 20-minute diagnose is arithmetic); a cold `cargo check` of `3-intelligence` on a Pi 5 and an N100; opencode's
  RSS on a hub; the hub's per-request RSS for a delegated diagnosis; the RSS of Netdata, monit, Prometheus and Cockpit (references only); the eval-budget
  and 64 GB fine-tune arithmetic.
- *Behaviour and platform:* whether opencode hides a denied tool or refuses it at call time; whether its bash tool needs `/bin/sh`; whether
  `OPENCODE_DISABLE_MODELS_FETCH` stops every start-up call; whether llama.cpp can serve on a unix socket; what hung in Layer 3's G2 observation;
  `NETLINK_KOBJECT_UEVENT` and inotify under Termux; `CONFIG_PSI` on Pi OS; `power_supply` uevents; Android 15 wake-lock kills; the Termux:API job-scheduler
  minimum; bubblewrap and Landlock on Android; unprivileged user namespaces per hub distro; any kernel free-space event; a non-zero stream id through the
  fragment and ACK paths (R2's test); whether the Android installer opens a file Termux wrote, and from which folder; Termux's own version read cheaply; the
  root helper honouring `revert-request.json` outside probation; the Junos, NETCONF, IOS-XR, LuCI and MikroTik defaults; the GitHub limits (80 a minute,
  500 an hour), fine-grained tokens on fork PRs, the issue-URL length cap and fork-workflow approval; Dependabot and Renovate PR limits.
- *Evaluations and incidents (all search snippets):* ITBench 11.4 % (v1 13.8 %), OpenRCA 11.34 %, OpenRCA 2.0 20.7 %, RCACopilot 0.766, log anomaly F1
  98.9/82.4, Huang et al., Self-Refine, SICA 17→53 %, ICSE 2026 7.8/29.6 %, SWE-ABS 19.78 %, SWE-bench+ 32.67/31.08 % (versions differ), GenProg 2/105 (Kali
  matched it), Google 15 %, Meta 25.5/80 %, CodeMender's 72 fixes, Sclar's 76 points, ACE, MINJA 76.8 %, AgentPoison, MemoryGraft, Shumailov, GEPA's gains,
  METR 1–2 %, Tam et al.; the DGM's fabricated log and removed markers; the AI Scientist's timeout edit; the Replit deletion (dated 07-18 and 07-23 in
  different reports); the Invariant Labs leak; the end of curl's bounty on 2026-01-31; the Copyright Office report; promptfoo's acquisition; Agent Skills
  adoption; in-toto in CNCF.
- *Legal and licences (not read from the upstream file):* systemd repo-wide; the crates `inotify`, `sd-notify`, `landlock` (crates.io metadata only);
  `antlr4rust` (BSD-3-Clause per crates.io); rpcd (a header only); Termux:API, termux-boot, termux-services; cronie; Cockpit; "or later" for ABRT, snapper,
  ostree and bubblewrap; Unsloth; OpenHands `enterprise/`; SICA; DEP-3; Icinga 2 (`COPYING` answered 404). The authorship and sign-off reading (03 finding 9)
  is not legal advice.
- *Repo (carried):* `build.py` inside `share/kb/`; the agent111 test counts; `proposals.rs:412-475`.
