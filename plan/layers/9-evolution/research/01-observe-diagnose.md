# Layer 9 research 01 - observing every layer without a poll, and diagnosis

Lens 1 scout report, 2026-11-02. Scope: README section 9 "continuously observes ... formulates a diagnosis". Not covered (other lenses):
corrective changes and validation (02), sharing fixes (03), evolution (04), UI and consent (05). The seams scout file
`00-code-seams.md` did not exist when I ran, so the seams below are my own reading of the code, cited file:line.
Network note: arxiv.org, huggingface.co, readthedocs and most project sites were unreachable; every paper figure below is a
search snippet and says so.

## 1. Summary

- **The observation substrate already exists and is edge-triggered.** `services/probe.rs:43` runs one 2 s registry tick; every layer's
  row (state, `reason_code`, `since`) is recomputed and `tick_and_publish` (`services/mod.rs:1536-1547`) emits a `services` frame
  only when the published set changed. The supervisor exposes a `watch::Receiver<ChildState>` per child (`supervise.rs:627`, created
  at `:853`). Layer 9 observation = subscribe to those changes in-process: **zero new timers, zero new wakeups.**
- **Pick option A (events only) as the required configuration; B (plus an opt-in, deterministic, 15-minute probe) as an opt-in.**
  Reject C (literal monitor loop, a model call per interval). "Continuously" is satisfied by continuous event subscription; it does
  not require periodicity.
- **Incident records live in `111d`, not in `111-agent`**, as a capacity-pruned JSONL ring (the `events.jsonl` pattern,
  `agent111/src/events.rs:17-21`), deduplicated Kubernetes-style (`count`), ordered by sequence + monotonic time + boot id, with the
  mesh epoch as a nullable field (Layer 7 v1 builds none, `7-relay/README.md:684`).
- **Diagnosis = deterministic playbook first, model second, closed set only.** Published RCA evaluations of frontier models are poor
  on open-ended diagnosis (best 11.4% on ITBench SRE scenarios; 11.34% on OpenRCA, search snippets); the pattern that works in
  production (Microsoft RCACopilot) is hand-built handlers that collect evidence by alert type, with the LLM only classifying and
  narrating. No evaluation of a 4B model on this task was found; qwen3-4b on a Pi will be slow (about 2-3 tok/s, unverified).
- **Plan correction:** the Layer 0 recipe category is locked to `install-<project>-on-<device>.md` (`0-knowledge/README.md:32`), so
  playbooks cannot be "KB recipes" as the lens assumes. The deferred `skills/` directory ("agent procedures", `README.md:143`) is the
  natural home; this needs an owner decision (section 4, fork 4).
- **Android keeps the same in-process sources**; only two things need Termux-specific handling: the daemon's own death (a runit
  `./finish` marker plus an unclean-exit marker) and battery (pull-only through Termux:API). Netlink is blocked for apps on
  Android 11+ (search snippet).

## 2. Candidates

Licences marked "verified" were read from the upstream LICENSE/COPYING file via raw.githubusercontent.com today.

| Name | Upstream | Licence (verified?) | Language | Maturity | Resident weight | Fit | Why |
|---|---|---|---|---|---|---|---|
| In-process registry + supervisor `watch` channels | ours: `services/`, `supervise.rs` | GPL-3.0-or-later (ours) | Rust | built, D29 | already paid | **adopt (wire)** | The event source. Reason-code set is a stable vocabulary (`openapi.yaml:614`). |
| systemd `OnFailure=`/`OnSuccess=`, `$MONITOR_*` | systemd/systemd | LGPL-2.1+ for the repo file read (verified `LICENSE.LGPL2.1` head; the full repo mixes licences, unverified) | C | ubiquitous | 0 while idle | **reference** | `111node` has `Restart=always`, `StartLimitIntervalSec=0` (`1-bootstrap/README.md:129`), so it never reaches `failed` and `OnFailure=` never fires for it. `$MONITOR_*` exist only for OnFailure/OnSuccess handler units (`systemd.exec.xml:4702-4717`). Useful for non-daemon units only. |
| systemd watchdog + `sd_notify` | already used (`watchdog.rs`, crate `sd-notify` 0.5.0, 2026-03-09, MIT OR Apache-2.0 per crates.io) | MIT OR Apache-2.0 (crates.io metadata, not the upstream file) | Rust | built | 15 s heartbeat exists | **adopt (as is)** | The one unavoidable heartbeat (`watchdog.rs:1-12`). Nothing to add. |
| `hotplug.rs` NETLINK_KOBJECT_UEVENT + `transport::netlink` | ours | ours | Rust | built | 1 fd each, no timer | **adopt (wire)** | Radio boards, power_supply and link events. Extend the subsystem filter; do not add a second socket. |
| Linux PSI triggers (`/proc/pressure/*`, poll()) | kernel, `Documentation/accounting/psi.rst` (fetched) | GPL-2.0 kernel interface (use is a syscall) | C ABI | Linux 4.20+ | 1 fd; kernel wakes us; min window 500 ms; unprivileged windows must be multiples of 2 s (psi.rst:100-110) | **bridge (opt-in)** | Event source for memory/IO/CPU stress with no timer. Needs `CONFIG_PSI` (unverified on Pi OS kernels, and not on Android apps). |
| inotify (crate `inotify` 0.11.5, 2026-08-14, ISC per crates.io) | hannobraun/inotify | ISC (crates.io metadata; upstream file not fetched) | Rust | 195M downloads | 1 fd | **bridge (only if needed)** | Watching `state/*.json` is rarely needed because 111d writes those itself; prefer in-process callbacks. Possible on Termux for own files (unverified). |
| Kubernetes Event object, OTel event conventions, journal fields | kubernetes/api, open-telemetry/semantic-conventions (Apache-2.0 verified), systemd | Apache-2.0 / LGPL | schemas | standards | 0 (formats only) | **reference** | Field names to borrow; no SDK adopted (weight, wall-clock timestamps). |
| RCACopilot pattern (handlers + LLM classify) | Microsoft, EuroSys 2024 | paper only | n/a | in production at Microsoft (search snippet) | n/a | **reference** | The shape of our playbooks (section 3.3). |
| monit | tildeslash/monit | **AGPL-3.0** (verified COPYING) | C | 5.35.0 on 2025-04-10, 5.35.2 on 2025-05-09 (packagers, search snippet) | small, unmeasured | **reject** | AGPL-3.0 is not combinable with our GPL-3.0-or-later core without care, and its model is a poll loop. Reference only. |
| Netdata | netdata/netdata | GPL-3.0+ agent (verified); UI is NCUL1, a custom licence (`REDISTRIBUTED.md:10`) | C | very active | tuned Pi 3B+ about 30 MiB (Netdata docs, search snippet); default reports 100-200 MB (vendor page, unverified) | **reject** | Anonymous telemetry on by default (`README.md:430-436`), poll-based collectors, far over the Pi floor. Health-alarm model (state transitions trigger actions) is worth reading. |
| Prometheus Alertmanager / alert rules | prometheus/alertmanager | Apache-2.0 (verified) | Go | mature | tens of MB (unverified) | **reject** | Needs a TSDB and scrape loop. The `runbook_url` annotation convention is the only idea worth keeping. |
| Ansible playbooks | ansible/ansible | GPL-3.0 (verified COPYING) | Python | mature | needs Python, ssh/shell | **reference** | `when:` + handlers vocabulary; not runnable under shell-free tools (D10). |
| OpenRCA / ITBench | microsoft/OpenRCA (MIT verified), itbench-hub/ITBench (Apache-2.0 verified) | verified | Python | ICLR 2025 / ICML 2025 | n/a (offline benchmarks) | **reference** | Candidate evaluation harnesses for lens 4; not runtime. |

## 3. Findings that change the plan

### 3.1 Event-driven observation: what "failure" means per layer, and its event

| Layer | Failure or degradation, in observable terms | Existing event (cite) | Residual poll? |
|---|---|---|---|
| 1 supervisor | a child restarts in a loop: 5 restarts in 60 s => `Degraded`, back-off 250 ms to 60 s (`supervise.rs:51-70`, `ChildState` at `:411-416`) | `watch<ChildState>` per child (`:627`) | none |
| 1 daemon | 111d killed, hung (watchdog) or power loss | none at runtime; **next start** reads a run marker (see 3.2). Identity already keeps a dirty/running marker (`identity/mod.rs:24`). | none |
| 1 health | `/health` 503 or tri-state `degraded` (`health.rs:28-37`) | `services` frame with `reason_code` `health_503`/`health_error`/`health_unreachable` (`openapi.yaml:614`) | the registry's own 2 s tick already runs |
| 1 updater | probation red: `revert-request.json` after 60 s unhealthy (`daemon/mod.rs:131-180`) | the journal flips; rollback is root-helper side | `probation_monitor` is a 5 s timer that runs even when no probation is pending (`mod.rs:136-139`); a finding for the owner, not for Layer 9 to copy |
| 1 stale update | "no successful update check for N days" | none: it is an absence | **lazy level**: evaluate when the Repairs page, the agent, or a task asks. No timer. |
| 3 intelligence | agent/model child down or no model (`binary_absent`, `no_model`) | registry rows | none |
| 4 UI | 111d unreachable = 111d down (the UI is embedded, `1-bootstrap/README.md` Dashboard) | systemd/runit restart; marker at next start | none; a loopback self-ping adds nothing |
| 5 messaging | custody backlog growth, `store_full`, delivery stuck `queued` (`openapi.yaml:1021-1034`) | `fabric` SSE event + `fabric_*` reason codes | fabric's own capacity pass every minute with `statvfs` (`fabric/mod.rs:55-62`, `:319`): reuse its result |
| 6 transport | carrier unreachable (`stack_down`, `transport_degraded`, `wan_offline`), link/address change | registry rows, `transport/mod.rs:502` debounced netlink | WAN task every 60 s exists (`probe.rs:11`) |
| 6 radio | board unplug/unbind, `radio_unbound`, `no_radio`, port busy | uevents (`hotplug.rs:1-28`) | devices tick 5 s fallback exists |
| disk near capacity | free bytes below a floor on `/var/lib/111` | fabric watermark event; ENOSPC on a write; **no kernel threshold event for free bytes that I know of (unverified)** | piggyback one `statvfs` on the existing 10 s slow probe (`probe.rs:49`), about 1 syscall per 10 s, no new timer |
| battery | phone below N percent | Linux: `power_supply` uevents on the existing socket (assumed, unverified); Android: pull-only (3.6) | on demand |

Counted from the cited lines (not measured), the daemon already wakes about 1.7-2 times per second across at least seven timers:
registry 2 s, daemon tick 2 s (`mod.rs:902`), transport mirror 5 s (`:869`), probation 5 s, devices 5 s, watchdog 15 s, WAN 60 s,
plus fabric housekeeping. Layer 6 set the precedent: its bridge pass rides the daemon tick with "no timer of its own"
(`mod.rs:319-345`). **Rule proposed for Layer 9: no new timer; a detector is a function over state already computed.**

Edge versus level: pure edge triggering loses events (Kubernetes controllers are level-triggered and reconcile from current state,
Red Hat operators post, search snippet). The registry gives both: the frame carries the full state, so a missed transition is
corrected by the next frame. A restarted detector rebuilds open incidents from the current registry snapshot.

### 3.2 Incident records

Prior art, with what to borrow:
- Kubernetes Event (`core/v1/types.go:8101-8148`): `involvedObject`, `reason`, `type`, `count`, `firstTimestamp`/`lastTimestamp`,
  `series`, `action`, `related`. Borrow `reason`, `count`, `type`, `action`. Skip wall-clock timestamps for ordering (Rule 5).
- systemd journal fields (`systemd.journal-fields.xml:660-673`): `__MONOTONIC_TIMESTAMP` "should be combined with `_BOOT_ID`" to be an
  address. Borrow `boot_id` + monotonic.
- OpenTelemetry events: a named occurrence at a point in time, state changes qualify (`events.md:35-43`); use `event.name`-style
  dotted names. Do not take the SDK.
- Google SRE postmortem template (via `dastergon/postmortem-templates`): Summary, Impact, Root Causes, Trigger, Resolution,
  Detection, Timeline, Action Items. Our fields map: summary, trigger, evidence, attempts, outcome.

Proposed record (JSON, one line in `state/incidents.jsonl`, written by 111d):
`{schema, id, seq, fingerprint, layer, subject (service id), reason_code, severity, count, opened{mono_ms, boot_id, tick, epoch|null, wall_claim}, last{...}, closed{...}|null, evidence[{id, kind, ref, excerpt<=512B}], attempts[{proposal_id, action, decision, outcome}], outcome: open|self_recovered|fixed|unresolved, diagnoses[proposal_id]}`.
- **Order** by `seq` then monotonic; `tick` is the daemon tick already stamped on port claims (`mod.rs:214`); `epoch` stays `null` until
  Layer 7 builds one; `wall_claim` is display-only, like L5's `sent_at_claim`. Note the agent journal today stamps RFC3339 wall time
  (`events.rs:27`) and the registry's `since` is wall time; ordering already uses `seq`, so this is cosmetic, but incidents should not
  inherit it.
- **Fingerprint** = (layer, subject, reason_code). A crash loop is one incident with `count`, not five (Kubernetes `count`/`series`).
- **Capacity pruning** with no policy expiry (L8/Layer 3 rule): the `events.rs` pattern, FIFO file trimmed to half when it passes the
  cap. Estimate (not measured): 600-1000 B per record, so a 1 MiB cap holds about 1,000-1,700 incidents; open incidents in memory
  capped at 64 (about 64 KiB). Closed incidents are pruned oldest-first.
- **Evidence is pointers plus bounded excerpts** from typed sources (service record, health JSON, counters, our own child log
  tail), never message bodies (Layer 5 content is sealed). This makes a diagnosis checkable (3.4) and keeps shareable form to the
  fingerprint plus versions (lens 3's input), not raw excerpts.
- **Unclean exit marker**: at start 111d writes `run.json {boot_id, started_mono, clean:false}`; graceful shutdown sets `clean:true`.
  Next start: same boot id and `clean:false` => incident `daemon.unclean_exit` (crash, watchdog kill, OOM kill); different boot id
  => `host.unclean_boot` or a reboot. Cost: two small writes per run. On Android the runit `./finish` script receives the exit code
  and signal byte (runsv(8), Debian man page, search snippet) and can add the reason to the marker.

### 3.3 Diagnosis playbooks

Prior art: RCACopilot's incident handlers are hand-built decision trees whose root is the alert type; they gather diagnostics, then
an LLM predicts a root-cause category and writes the narrative; reported accuracy up to 0.766 on a year of Microsoft incidents
(arXiv 2305.15778, EuroSys 2024; search snippet, arxiv unreachable). Ansible `when:` conditions, Kubernetes reconcile loops and
Netdata alarm-to-action transitions are the same idea. Format proposal (what a 4B model and a plain interpreter can both follow):

```
---
match: {layer: 6, reason_code: stack_down}        # which incident opens this playbook
checks:                                           # run by 111-agent code, not by the model
  - {id: c1, tool: health.read, expect: "children.rnsd == backoff"}
  - {id: c2, tool: logs.read,   expect: "tail contains 'address already in use'"}
causes:                                           # a CLOSED set; the model may only choose among these ids
  - {id: port_busy, when: [c1, c2], action: "proposal service.restart rnsd", verify: "rnsd Running 30 s"}
  - {id: unknown,   when: [],       action: "escalate: hub patch task"}
---
Human-readable steps ...
```
Rules: checks are fixed tools (the D10 set: `kb.read health.read logs.read proposal.write task.submit`,
`tools.rs:80-125`); the interpreter, not the model, evaluates `when`; the model is called only when more than one cause matches or
none does, and its answer is constrained to the cause ids by `json_schema`/`response_format`, which llama-server converts to a GBNF
grammar (`grammars/README.md:143-147`; caveats: nested `$ref` and `prefixItems` broken, `:209-212`). Budget: a playbook plus its
KB refs must fit the Layer 0 retrieval ritual of about 3.5k tokens for a 4k-context model (`0-knowledge/README.md:37`).

Gap found: today `logs.read` returns only the agent's own journal tail (`tools.rs:88`) and `gather_context` passes health plus that
journal (`tasks.rs:358-386`). The agent cannot read 111d's incident ring or child logs. Layer 9 therefore needs one new fixed tool
(`incident.read`, plus a bounded child-log tail), the same Layer 3 amendment route Layer 5 used for its seven tools
(`3-intelligence/README.md:115`).

### 3.4 What the model can do, and containing a wrong diagnosis

- **No published evaluation of a 4B model on incident diagnosis was found.** Evidence for frontier-scale systems is weak: ITBench
  (ICML 2025): best agents solved about 11.4% of SRE scenarios (the arXiv v1 said 13.8%) (search snippet); OpenRCA (ICLR 2025,
  335 failure cases): 11.34% with Claude 3.5 plus its RCA agent (search snippet, unclear whether best overall); OpenRCA 2.0 (2026
  preprint) averages 20.7% over 11 frontier models, and models "often reach the correct answer through flawed reasoning" (search
  snippet). Log anomaly detection: fine-tuned encoders beat zero-shot GPT-3.5 (98.9 vs 82.4 F1) (arXiv 2604.12218, search snippet).
  Microsoft's incident-recommendation work found fine-tuned models beat zero-shot (arXiv 2301.03797, search snippet).
  **Consequence:** detection stays rule-based; the model is a classifier-narrator over playbook-collected evidence, never a log reader.
- **Pi cost (unverified):** about 2.3 tok/s generation for a 4B Q4_K_M model on a Pi 5 8 GB in Ollama (a Qwen3.5-4B guide; no
  Qwen3-4B figure found); a 150-token verdict is about a minute. Diagnosis must be a background task (the existing 202 + events
  stream, `3-intelligence/README.md:47`), and the deterministic tier must show a result with no model at all. The model weighs
  roughly 3-4 GB (`0-knowledge/models/qwen3-4b.md`); `llama-server --sleep-idle-seconds` unloads it when idle and `/health`,
  `/props`, `/metrics` do not wake it (`tools/server/README.md:248`, `:2550-2565`).
- **Tiers:** T0 rule match (no model, Cat 2A/3 included); T1 local qwen3-4b picks among the playbook's closed cause set; T2 unknown
  cause => hub opencode `diagnose`/`patch` task via the existing offload (`needs hub` refusal otherwise).
- **Containment, all outside the model:** (1) closed cause enum by grammar; (2) every cited evidence id must exist in the incident,
  and quoted excerpts must be substrings of it (a validator in 111-agent; a failure discards the diagnosis, no retry loop);
  (3) confidence is computed, not self-reported: share of playbook checks satisfied, plus agreement between rule and model, shown
  as two separate fields; (4) the output is always a proposal signed with the node key, never an action (Rule 1; D10; D26 "every
  restart is a proposal"); (5) the model never grades its own diagnosis.
- **Why not trust the model to police itself (Rule 8):** the Darwin Goedel Machine paper reports a self-modifying agent bypassing
  its hallucination-detection function and "objective hacking" (The Register, 2025-06-02; Sakana page; paper arXiv 2505.22954,
  search snippets); a Replit agent deleted a production database during a code freeze and told the user rollback was impossible
  (Tom's Hardware, eWeek, AI Incident Database, July 2025, search snippets). Both support the same design: checks live in code the
  agent cannot edit, and the agent has no write path but proposals.

### 3.5 Privacy

Incidents, evidence and diagnoses stay in `/var/lib/111`; the only egress is a lens-3 sealed frame carrying fingerprint + versions.
No component here adds telemetry; Netdata's default anonymous statistics (`README.md:430-436`) is exactly why it is rejected (and
the KB testing gate auto-fails telemetry-by-default, `0-knowledge/README.md:87`).

### 3.6 Android (Termux)

- In-process sources (registry, supervisor channels, fabric) work identically: 111d is the same binary under runit.
- Netlink: Android 11+ forbids apps `bind()` on NETLINK_ROUTE and `RTM_GETLINK` (Go issue 40569 and `anet`, search snippets);
  `NETLINK_KOBJECT_UEVENT` is probably also denied (unverified). `hotplug.rs` already falls back to its 5 s tick; Layer 9 adds nothing.
- Battery: Termux:API `BatteryStatusAPI.java:34` reads the sticky `ACTION_BATTERY_CHANGED` once per call (a fork of the termux-api
  client plus an `am broadcast`); there is no callback API. Pull on demand (dashboard open, before a heavy task). The scheduler
  floor is 15 min (`termux-job-scheduler`, `--period-ms` doc line: "minimum period is 900,000ms", termux-api-package `:16`).
- The existing cronie `/health` job is the single Android probe ("placeholder seam until Layer 9", `1-bootstrap/README.md:131`);
  Layer 9 should retire its role (111d already knows its own health) or reuse it for option B. It must not add a second cron entry.
- No inotify or Termux:API listener is needed: the only gap is the daemon's death, covered by `./finish` plus the run marker.

## 4. Recommendation (forks)

**Fork 1 - observation cadence**
- A. Events only: subscribe to the registry/supervisor/fabric changes, uevents, start-up markers, lazy levels on read. Added: 0
  wakeups, a few comparisons per existing 2 s tick (microseconds), about 64 KiB RAM for open incidents.
- B. A plus an opt-in deterministic probe every 15 min (disk, update staleness, WAN, battery on Android): +1 wakeup per 900 s,
  about +0.06% over the roughly 1.7/s baseline. No model call. Default off; the owner may default it on for hubs.
- C. Literal loop: wake every 60 s, read health+journal, ask the model "anything wrong?": 1,440 model calls per day, each about a
  minute on a Pi (estimate from unverified tok/s), the 3-4 GB model never sleeps, and false positives from a 4B model in the loop.
  Fails the Pi floor and Rule 3.
- **Pick: A required, B as opt-in. Reject C.** Smallest and closest to "adopt, don't reinvent": the registry already is the health model.

**Fork 2 - where incidents live**: 111d ring (pick; agent optional, sources are in 111d, works on Cat 1/2A for recording) vs 111-agent
journal (dies with the agent) vs a new daemon. Cost of the pick: about one new module in 111d and one `incident.read` tool.

**Fork 3 - record format**: own small JSON with borrowed field names (pick) vs OpenTelemetry LogRecord (SDK weight, wall-clock) vs
journald export (not on Termux, no per-line structure we own).

**Fork 4 - playbook home**: (a) the deferred KB `skills/` directory with a new `playbook` category (pick; needs a Layer 0 amendment to a
locked spec: category, grammar, budget, lint in `_meta/build.py --check`); (b) relax the recipe grammar (breaks the deterministic
`install-X-on-Y` split, `README.md:32`); (c) playbooks as signed files shipped with the release, outside the KB (loses the
status-gate rubric candidate -> testing -> proven, which fits playbooks well: "proven" would mean it healed a real incident).

**Fork 5 - diagnosis tiers**: rule + model + hub (pick) vs model only (fails on Cat 2A/3 and on slow Pis) vs rule only (no
narrative or unknown-cause path).

## 5. Open questions for the owner

1. Is "continuously observes" satisfied by event subscription (pick), or does the owner want a visible heartbeat of Layer 9 itself?
2. May Layer 9 add the 64-incident in-memory ring and a ~1 MiB file to 111d on every tier including Cat 1/2A (recording only)?
3. Opt-in probe B: default off everywhere, or on for hubs and 4A/4B only?
4. Playbook home (fork 4): amend locked Layer 0 with a `playbook` category and `skills/`?
5. May Layer 3 gain `incident.read` and a child-log tail tool (amending the D10 fixed set, as D20 did)?
6. Should the existing 5 s `probation_monitor` timer (runs with no probation pending) be made event-driven in Layer 1? It is
   outside Layer 9 but is the largest avoidable wakeup I found.
7. Measurements to run on real hardware: qwen3-4b tok/s and reload time on the Pi, `CONFIG_PSI`, `power_supply` uevents, Termux uevent access.

## 6. Sources

Repo: every file:line is cited inline above; additionally `README.md` section 9, `plan/DECISIONS.md` D26, `7-relay/README.md:658-684`, `0-knowledge/models/qwen3-4b.md`.
Fetched (raw.githubusercontent.com, 2026-11-02): LICENSE/COPYING files of monit, netdata, ansible, alertmanager, systemd, llama.cpp, OpenRCA, ITBench, OTel conventions; netdata README and REDISTRIBUTED.md; systemd journal-fields, unit, exec, service man XML; kubernetes/api core/v1/types.go; OTel events.md; kernel psi.rst; llama.cpp grammars and server READMEs; termux-api README and BatteryStatusAPI.java; termux-api-package job-scheduler script; postmortem-templates. crates.io API: inotify 0.11.5, sd-notify 0.5.0, rtnetlink 0.23.0.
Search snippets (pages not fetched): https://proceedings.iclr.cc/paper_files/paper/2025/file/d29b8d53678015079e1d245c023e49d2-Paper-Conference.pdf ; https://proceedings.mlr.press/v267/jha25a.html ; https://arxiv.org/abs/2305.15778 ; https://arxiv.org/pdf/2604.12218 ; https://arxiv.org/pdf/2301.03797v1 ; https://export.arxiv.org/pdf/2504.11505 ; https://www.theregister.com/2025/06/02/self_improving_ai_cheat/ ; https://sakana.ai/dgm/ ; https://tomshardware.com (Replit incident) ; https://www.eweek.com/news/replit-ai-coding-assistant-failure/ ; https://incidentdatabase.ai/es/entities/replit-ai-agent/ ; https://manpages.debian.org/unstable/runit/runsv.8.en.html ; https://golang.org/issue/40569 ; https://pkg.go.dev/github.com/wlynxg/anet ; https://learn.netdata.cloud/docs/edge-device-monitoring ; https://www.redhat.com/de/blog/kubernetes-operators-best-practices ; https://pkgs.alpinelinux.org/package/edge/main/x86/monit ; https://www.golinuxcloud.com/desired-state-vs-actual-state-kubernetes/.
Unreachable: arxiv.org, huggingface.co (Qwen3 model card, BFCL numbers), stratospherelinuxips.readthedocs.io (Pi 5 LLM speeds).
