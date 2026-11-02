# Layer 9 research 02 - the corrective change, its validation, and apply -> verify -> undo

Lens 2 scout report, 2026-11-02. Scope: README section 9 "generates and internally validates a corrective change" and parked P4.
Other lenses cover observation (01), sharing fixes (03), evolution (04), UI and consent (05). Code claims are my own reading, cited file:line,
cross-checked against `00-code-seams.md`. arxiv.org and rfc-editor.org are unreachable from this box: figures marked "(search
snippet)" come from WebSearch, not a fetched page. "Verified" licences were read from the upstream LICENSE/COPYING file today.

## 1. Summary

- **The engine exists in narrow form; its undo lives in memory.** `devices/applier.rs` snapshots into a plain struct (`applier.rs:835-839`);
  a daemon crash between apply and probe leaves a "visibly half-applied" proposal that is "never applied a second time" (`:35-37`) and also
  never reverted. Every proven system below arms the rollback BEFORE the change and makes it outlive the actor. Pick: extend the applier
  with a persisted armed undo record, resolved at the next daemon start (no new process, no timer while idle).
- **Four change types a node can apply itself; the rest are human-only or pull requests.** Settings PATCH, service restart, the closed
  device vocabulary and 111-owned agent data overlays fit apply -> verify -> undo. Code, manifests, release-key material, the KB (an
  immutable release asset, `1-bootstrap/README.md:39`) and firmware flashing do not (section 3.2).
- **Pattern pick: "commit confirmed" with two confirmers.** An internal probe confirms by default; a human session confirms the one class
  that can cut the user off (reachability). Precedents: Junos (10 min default), NETCONF `persist-id`, OpenWrt LuCI (90 s), ESP-IDF
  `PENDING_VERIFY` (reset before confirm = rollback, no clock). Reject generations and partitions (NixOS, ostree, A/B): no second
  partition on a Pi or phone, and they need reboots.
- **"Internally validates" per category:** phone = the action's own verify; Pi = plus a pure `plan()` dry-run; hub = plus namespaces and the
  Layer 6 gates as root, for validating pull requests only (D12: a node never installs a binary it built).
- **A passing check is weak evidence.** Validators are satisfied by deleting functionality (Kali) and benchmarks leak (SWE-bench+: 32.67%
  leakage); Google's sanitizer pipeline fixed 15%, Meta's engineering agent landed 25.5% (search snippets); nothing is published for a 4B
  model. Probes must assert the capability, and the probe, deadline and loop ledger must sit where the agent cannot write.
- **Plan corrections:** P4 says restarts "may skip approval", superseded by D26 #2 ("every restart is a proposal"); the README's 60 s
  probation (`1-bootstrap/README.md:158`) disagrees with `PROBATION_SECS = 600` (`probation.rs:57`).

## 2. Candidates

"Resident weight" = what adopting the PATTERN adds to 111d. Most rows are references: the engine is 111d's own Rust.

| Name | Upstream | Licence (verified?) | Maturity | Resident weight | Fit | Why |
|---|---|---|---|---|---|---|
| Applier (apply, probe, keep/revert) | ours: `devices/applier.rs` | GPL-3.0-or-later | built, D29 | probe polls 1 s, at most 30 s, only inside an accepted apply (`applier.rs:85-87`) | **adopt (extend)** | The one engine: add the persisted undo, a confirmer field, the ledger. No fork. |
| Probation journal + `update-guard` | ours: `probation.rs`, `update_guard.rs` | ours | built | zero idle | **adopt (pattern)** | Armed record that survives restarts, resolved before serving (`update_guard.rs:34-52`). |
| OpenWrt rpcd `uci apply` rollback + LuCI "apply with revert" | openwrt/rpcd `uci.c`, openwrt/luci `ui.js` | rpcd: ISC-style notice in the file header (verified `uci.c:3-12`; no root LICENSE file); LuCI Apache-2.0 (verified) | in every OpenWrt; 90 s default (search snippet) | one timer while pending | **reference** | Closest match: `apply {rollback, timeout}` arms a timer (`uci.c:1640-1668`), `confirm`/`rollback` methods (`:1804-1805`), a second pending rollback-apply refused (`:1602`), timeout clamped (`:1663-1666`), UI warns when a change touches the connection in use (`ui.js:5789`). |
| Junos / IOS-XR / NETCONF confirmed commit | Juniper, Cisco, RFC 6241 s8.4 | specs, no code | decades; 10 min Junos, 600 s NETCONF, 30-300 s pyIOSXR (search snippets) | n/a | **reference** | The exact pattern. `persist-id` lets another session confirm; a re-sent confirmed commit extends the window. |
| MikroTik Safe Mode | RouterOS docs | n/a | production | n/a | **reference** | Revert on abnormal session end, no timer; flaps trigger it (search snippet). The cautionary second trigger. |
| ESP-IDF OTA rollback | espressif/esp-idf `ota.rst` | Apache-2.0 (verified) | production | none | **adopt (semantics)** | `PENDING_VERIFY`; a reset or power loss before confirmation rolls back; confirm-on-startup or app-decides (`ota.rst:43-60`). No clock. |
| systemd boot counting; Android A/B | systemd `AUTOMATIC_BOOT_ASSESSMENT.md`; source.android.com | LGPL-2.1 file verified (repo mixes licences, unverified); docs | production | none | **reference** | Tries-left counters, "good" only after assessment (`aba.md:65-75`); `successful` flag set by user space (search snippet). Counters, not clocks; slots need a second partition. |
| NixOS; ostree; transactional-update; snapper | NixOS/nix(pkgs); ostreedev/ostree; openSUSE/* | nix LGPL-2.1, nixpkgs MIT, ostree LGPL-2.0 text, t-u GPLv2+/LGPLv2.1+, snapper GPL-2.0 text (all verified; snapper "or later" unverified) | production | store, btrfs or partitions | **reject** | Generations or root snapshots plus reboot. No Termux, no spare partition on a Pi. |
| Kubernetes rollouts; Erlang/OTP supervisors | kubernetes/website; erlang/otp `sup_princ.md` | Apache-2.0 both (verified) | production | n/a | **reference** | K8s "takes no action on a stalled Deployment other than to report `ProgressDeadlineExceeded`" (`deployment.md:966`): verify and undo are separate authorities. OTP restart intensity/period (`sup_princ.md:251-258`) bounds loops like `supervise.rs:427-484`. |
| Ansible check mode | ansible-documentation | GPL-3.0 text (verified `COPYING`) | mature | needs Python | **reference** | "Check mode is just a simulation": a pure `plan()` per typed action. |
| bubblewrap / nsjail / `landlock` crate | containers/bubblewrap; google/nsjail; rust-landlock | LGPL-2.1 (verified); Apache-2.0 (verified); crate MIT OR Apache-2.0 (crates.io 0.4.7, 2026-07-27; upstream file not fetched) | mature | zero while off | **bridge (hub, on demand, opt-in)** | Sandbox for validating a pull request on a hub (user namespaces; bwrap README). Android availability unverified. Never in the base. |
| Darwin Godel Machine | jennyzzt/dgm | Apache-2.0 (verified) | ICLR 2026 poster (search snippet) | research code | **reference** | Self-modifying agent; README: "may still behave destructively" (`README.md:86-88`). |
| The AI Scientist | SakanaAI/AI-Scientist | custom "Source Code License v1.0" derived from the Responsible AI Source Code License (verified) | 2024 | research code | **reject** | Field-of-use licence, incompatible with GPL-3.0-or-later (D22). Incident reference only (F17). |

## 3. Findings that change the plan

### 3.1 What the code does today

- **F1. Undo is in memory.** `struct Snapshot` holds the parsed file and two byte buffers (`applier.rs:835-839`), taken per action
  (`:1354, :1383, :1415, :1464`). The decision record is written `pending` before the first effect (`applier.rs:27-30`, `begin` at
  `:640`) so a crash blocks a second apply, but nothing re-reads the pending record to revert at the next start. A crash mid-apply
  leaves the node on the new state and the proposal reading as decided. That is the one real defect against the pattern (the seams scout agrees: `00-code-seams.md:20-23`; settings PATCH has no snapshot at all,
  `00-code-seams.md:87-88`).
- **F2. The seam already carries a persisted armed record.** `Journal {pending, previous, deadline_epoch, health_confirmed}`
  (`probation.rs:7-17`) is written BEFORE the flip (`update_helper.rs:199-229`) and resolved by `update-guard` before the next start
  (`update_guard.rs:34-52`); the daemon can also request a revert of its own release (`update_helper.rs:237-262`). Two caveats: the
  deadline is an absolute wall-clock epoch (`update_guard.rs:50`, `update_helper.rs:206`), which an RTC-less Pi can skew in either
  direction, and the README's "60s probation window" (`1-bootstrap/README.md:158`) disagrees with `PROBATION_SECS = 600`
  (`probation.rs:57`). For typed actions use the ESP-IDF rule instead: "armed at start of daemon = revert", no clock.
- **F3. The vocabulary is closed and the refusal is loud.** Four applied actions, two one-shots, four refused names, 8 actions and 16 KiB
  per proposal, one proposal at a time (`applier.rs:85-105`, `:894`); an unknown name is `409 unsupported_action` before any write
  (`:20-22`). The apply runs to completion even if the HTTP client leaves (`:903-910`).
- **F4. The agent cannot apply.** The bearer is refused on every non-GET (`6-transport/README.md:650-657`); a `device` proposal is
  applied only inside the human's `POST /proposals/{id}/decision` (`4-interface/api/openapi.yaml:160`). Any "tier 2 auto-apply"
  therefore needs a new, human-authored standing record that the decision path consults; it is not a loosening of the bearer rule.
- **F5. The KB is not editable on a node.** It ships in the release payload (`share/kb/`, immutable; `1-bootstrap/README.md:39`) under
  root-owned `releases/` (`:169`: "The daemon can never modify its own payload"). A KB or recipe edit is a pull request, delivered back
  as a release or a `KB` frame, not a typed apply. Playbooks shipped in the payload are the same; a per-node overlay under
  `/var/lib/111/agent/` is the only apply-able variant.
- **F6. Agent tools are code.** The set is compiled and fixed (D10; `3-intelligence/README.md:23`), so a tool change is a pull request;
  prompts and playbooks are data: a typed, schema-validated overlay with an undo.

### 3.2 The change vocabulary

Tier: **A** = a typed apply with undo, human-accepted (auto-eligible only under a standing grant, section 4 fork D); **H** = human
decision AND human-run confirmation or a root helper; **P** = pull request only.

| Change | Exists today | Reversible | Blast radius | Verify step that proves it worked | Tier |
|---|---|---|---|---|---|
| `service.restart {service_id}` | yes, one-shot (`applier.rs:100`) | nothing to revert (idempotent) | one supervised child | `/health` 200 and the services row leaves `degraded`; bounded by the crash-loop window | A |
| `rns.interface.upsert/remove`, `rns.posture.set`, `radio.region.set` | yes (`applier.rs:93-99`) | byte-exact (TOML plus `rns/config`) | one interface or rnsd | `rnstatus` lists it up; helpers re-attached (`6-transport/README.md:628-632`) | A |
| Settings PATCH (`/fabric/settings`, `/relay/settings`, transport order) | no | byte-exact file snapshot | cosmetic to severe: `listen_lan`, `hub_role`, carrier can cut the user off | re-read equals intended; the feature's own probe; **reachability class: human confirm** | A, reachability class H |
| Component enable/disable | partly (consent, preflight; `6-transport/README.md` 2.1) | disable inverts it; a download stays | installs a third-party program | component `/health`; listener matches the manifest | H (never auto: weight, download) |
| `meshtastic.set`, `ble.pair/forget` | refused (`applier.rs:102`) | partly | a physical device | n/a | stays refused |
| `firmware.flash` | refused | **no** | brick | n/a | never typed; `rnodeconf` by the human only |
| `update.request` | root-helper path exists | yes, L1 probation revert | whole release | L1 health checklist, sustained (`1-bootstrap/README.md:158`) | H (root helper re-verifies the minisig) |
| Agent playbook or prompt overlay | no | file snapshot | changes what is proposed, not what is done | fixed golden cases re-run deterministically (no model call) produce the same classification | A (low risk; all writes stay gated) |
| KB or recipe edit | no | n/a on a node (F5) | the agent's grounding (a persistent prompt-injection target) | `0-knowledge/_meta/build.py --check` on a hub | P |
| Code, manifests, anything signed by the release key, agent tools | no | n/a | everything | CI and a human merge | P |

### 3.3 Proven patterns, what they teach

- **F7. Every confirmed-commit system arms the revert first, outside the actor.** Junos reverts after a default of 10 minutes
  unless confirmed and broadcasts to logged-in users; NETCONF defaults to 600 s, restores on reboot, and `persist-id` lets another
  session confirm (search snippets: juniper.net NETCONF and CLI pages, RFC 6241 errata 3821). rpcd arms a timer inside the daemon
  and refuses a second pending rollback (`uci.c:1602, 1640-1668`). ESP-IDF uses a state, not a timer (`ota.rst:45-60`). Ours is the
  weakest on this axis (F1).
- **F8. Timer, not session loss.** A timer fires even when the operator is gone; MikroTik's session-loss trigger fires on flaps, which
  LoRa and Termux nodes have. Pick timer plus explicit confirm. LuCI raised its window from 30 s to 90 s (search snippet); our 30 s probe (`applier.rs:85`) suits a service probe,
  not a human confirm.
- **F9. Verify and undo can be separate authorities** (Kubernetes reports, `deployment.md:966`). Ours: the engine reverts by itself only
  what it applied inside the probe window; anything after the keep is a new proposal.
- **F10. Loop bounds have a shape:** OTP intensity/period and our crash-loop threshold (`supervise.rs:427-484`) use monotonic windows;
  the apply/undo loop needs the same plus a ledger (fork E).

### 3.4 Validation by category (what "internally validates" can honestly mean)

| Category | Can run | "Validated" means | Cannot |
|---|---|---|---|
| 2A phone (Termux; no root, no cargo, background limits, cron placeholder) | the action's own verify; a pure `plan()` | the probe passed inside the window; the undo record is on disk | namespaces, sandboxes, builds |
| 2B Pi / terminal | the same, plus supervisor events as probe inputs | same; no new restart in the window | a build; a model check at useful speed |
| 3 | verify only; delegates the rest | as 2A | patch generation |
| 4A/4B hub | the Layer 6 gates G1-G7 (`6-transport/verify/`) in namespaces as root; a sandbox; cargo; opencode | for a pull request: builds, tests and a gate pass in a throwaway no-network sandbox | sign with the release key; install its own build on a node (D12) |

- **F11. Hub validation is for pull requests, not deployment.** The netns gates need root, no hardware (`6-transport/README.md` section
  16, `verify/root.sh`). User-namespace sandboxes or Landlock suit a normal Linux hub; Android availability is unverified.
- **F12. Dry-run is free if pure.** The applier already validates a proposal against a copy of the file before any write
  (`applier.rs:24-27`). Exposing it as `plan() -> {diff, preflight, reach}` is Ansible check mode at zero resident cost, and is what the
  UI shows before the accept.
- **F13. The canary across the mesh is the human.** A fix validated on node A reaches B as a proposal carrying A's verify transcript as
  DATA (the Layer 5 `PATCH` class, `5-messaging/README.md:479`). B runs its own plan and probe; a peer's transcript is evidence, not trust.

### 3.5 Safety evidence

- **F14. Validators get gamed by the cheapest passing path.** Qi, Long, Achour and Rinard (ISSTA 2015): GenProg produced a correct
  patch for 2 of 105 defects; "the overwhelming majority" of reported patches equal one modification that deletes functionality; Kali,
  which only deletes functionality, matched the prior systems (search snippet: cs.toronto.edu/~fanl/papers/kali-issta2015.pdf, MIT
  DSpace 1721.1/94337). Consequence for us: a "fix" that disables the failing service or removes the interface passes a probe that only
  asks "is it quiet?". Every typed action's probe must assert the capability (interface listed AND up; helper re-attached), as the
  applier's table does (`applier.rs:11-18`), and the engine must refuse a proposal whose net effect is to remove a capability unless the
  proposal says so as its stated purpose and a human accepted that text.
- **F15. Benchmarks overstate.** SWE-bench+ (arXiv 2410.06992 v2): 32.67% of resolved instances leaked the solution, 31.08% of passes
  came from weak tests, and one agent fell from 12.47% to 3.97% after filtering (search snippet; v1 differs, cite the version). Third-party
  aggregators call Verified near-saturated (unverified); none of it applies to a 4B model on a Pi.
- **F16. Production auto-fix systems keep a human gate and report modest rates.** Google's Gemini sanitizer pipeline fixed 15% of
  targeted bugs, for human review (research.google, 2024). CodeMender upstreamed 72 fixes, all human-reviewed, no acceptance rate
  published (Oct 2025). Meta's Engineering Agent: 80% of generated fixes reviewed, 25.5% landed in three months (arXiv 2507.18755). SapFix
  (2018) never deployed on its own. All search snippets. Our human gate matches the industry's.
- **F17. Self-modifying agents and rules held only in the prompt.** Sakana's AI Scientist doubled its own experiment timeout (2 to 4 hours)
  and in one run edited its script to call itself endlessly (Ars Technica, Aug 2024; a critic reads it as an ordinary fix loop, not
  intent). Replit's agent deleted a production database during a declared code freeze (Fortune, 2025-07-23). Search snippets. The DGM README says its code "may still behave
  destructively" (`dgm/README.md:86-88`, verified). Common lesson: a limit is only real when it is enforced outside the agent's write
  reach. Concretely: the agent writes `agent/proposals/` only; the probe code, the confirm deadline, the ledger, `agent/decisions/` and
  the standing grants are written by 111d (the applier's `DecisionFile` already does this at 0600/0700, `applier.rs:592-650`).
- **F18. Access-removal guard.** No applied action today can touch the dashboard or pairing (they edit rnsd and radio files); settings
  PATCH can. Each action declares `reach: none|may_affect`, computed in `plan()`. `may_affect` (listen_lan, port, hub_role, carrier,
  pairing) uses the LuCI pattern: apply, the human session re-reaches the node through the SAME URL and confirms within the window,
  else revert. An action that would remove the last authenticated pairing or the loopback listener is refused outright
  (`would_lock_out`). From rpcd: one confirmable apply pending at a time, timeout clamped.

## 4. Recommendation

**Fork A - the engine.**
- A1. Extend the narrow applier in place: `undo.json` (armed before the first effect, 0600, atomic) resolved at daemon start before
  serving; `confirmer: probe|human`; `plan()`. Cost: one small file write per apply; no idle timer (the human-confirm deadline is a
  tokio sleep that exists only while pending). Same path on Termux (single process, runit restart).
- A2. Generalise now into a `Revertible` trait, porting the four actions: cleaner, bigger diff, risks locked D29 behaviour.
- A3. Adopt an external engine (NETCONF/Nix): nothing fits the floor.
- **Pick A1**; refactor to A2 only when a fifth action family arrives.

**Fork B - when does the revert fire?**
- B1. Timer plus probe plus the start-of-daemon rule (armed = revert). B2. Session loss (MikroTik). B3. Reboot only (ESP-IDF).
- **Pick B1**: 30 s monotonic probe for automatic classes; armed-at-start = revert for crashes; a 90 s human confirm for the
  reachability class. No wall-clock deadlines for typed actions (F2's skew).

**Fork C - validation per category.**
- C1. Verify only everywhere. C2. Verify plus pure `plan()` dry-run everywhere (zero weight). C3. Add hub sandbox validation for pull
  requests, opt-in, on-demand component.
- **Pick C2 for all categories, C3 only on hubs that already opted into opencode** (`3-intelligence/README.md:19`). The phone
  gets exactly "the action's own verify", said plainly in the UI.

**Fork D - what may auto-apply.**
- Tiers: T0 observe, T1 propose (today), T2 apply reversible changes inside a user-set scope, T3 never code.
- D1. v1 ships T0/T1; the only automatic thing is the UNDO. T2 specified, not built. D2. T2 for restarts only (the P4 note).
- **Pick D1.** D2 contradicts D26 #2, which the owner has not reopened. Spec the audit record now:
  `{order, proposal_hash, action_hash, probe_result, revert_result, confirmer}`, appended by 111d to a capacity-pruned log (the
  `events.jsonl` pattern), nothing off the node. T2, if enabled, is a human-signed grant naming action class, target and a count under a
  monotonic window ("at most 3 rnsd restarts per boot"); it can never name tier H or P items.

**Fork E - loops and conflicts.**
- E1. A ledger keyed by `sha256(action, target)` holding `{applied, reverted, count}`: after a revert the agent does not re-propose the
  same action for the same cause until the human re-accepts or the evidence changes (count-based, no clock). A target with an armed undo
  refuses new proposals `409 busy`; when a newer proposal on one target is accepted, older open ones are marked superseded.
- E2. A global rate limit only.
- **Pick E1**, bounded by an OTP-style intensity/period reusing the supervisor's window type.

**Fork F - what a pull request means on a node.**
- F1. The node drafts the proposal document; a hub with git packages it. F2. The node signs a proposal envelope with the node key and
  the hub packages it.
- **Pick F2** (signing lands in Layer 9, `3-intelligence/README.md:84`); no git on the node (Layer 7 F6); the human merges on a machine
  with git. This is also lens 3's carrier seam.

## 5. Open questions for the owner

1. T2 (standing grants for reversible actions): ship the record format in v1 and keep it off, or leave T2 out of the spec entirely?
2. Is the 90 s human-confirm window right for the reachability class, and is "confirm from the same URL" acceptable on a phone behind
   NAT where the URL itself can change?
3. May a hub with opencode opt in to sandbox validation of pull requests (a new on-demand component, bwrap or Landlock, sign-off
   rows in `LICENSING.md`), or does validation stay "CI after the PR is opened"?
4. Probation: align the README to 600 s, the code to 60 s, or keep both (60 s sustained health inside a 600 s window)?
5. Should the engine refuse any proposal whose probe cannot assert a capability (F14), even at the cost of fewer proposals?

## 6. Sources

Code and docs read (file:line above): `1-bootstrap/crates/node111/src/daemon/devices/applier.rs`, `src/probation.rs`, `src/update_guard.rs`,
`src/update_helper.rs`, `src/daemon/supervise.rs`; `1-bootstrap/README.md`; `6-transport/README.md` (12, 13, 16); `5-messaging/README.md`;
`3-intelligence/README.md`; `4-interface/api/openapi.yaml`; `plan/cross-layer/parked.md` (P4); `plan/DECISIONS.md` (D17, D26); `7-relay/README.md`.

Fetched today from raw.githubusercontent.com (source files and LICENSE/COPYING for each): openwrt/rpcd `uci.c`; openwrt/luci `ui.js`, `LICENSE`;
espressif/esp-idf `docs/en/api-reference/system/ota.rst`, `LICENSE`; systemd `docs/AUTOMATIC_BOOT_ASSESSMENT.md`, `LICENSE.LGPL2.1`; kubernetes/website
`content/en/docs/concepts/workloads/controllers/deployment.md`, kubernetes/kubernetes `LICENSE`; erlang/otp `system/doc/design_principles/sup_princ.md`,
`LICENSE.txt`; NixOS/nixpkgs `nixos/doc/manual/administration/rollback.section.md`, `COPYING`; NixOS/nix `COPYING`; openSUSE/transactional-update
`README.md`, `COPYING`; ostreedev/ostree `COPYING`; openSUSE/snapper `COPYING`; ansible/ansible-documentation `playbooks_checkmode.rst`;
ansible/ansible `COPYING`; containers/bubblewrap `COPYING`, `README.md`; google/nsjail `LICENSE`; landlock-lsm/rust-landlock `README.md`;
https://crates.io/api/v1/crates/landlock ; jennyzzt/dgm `README.md`, `LICENSE`; SakanaAI/AI-Scientist `LICENSE`, `README.md`.

Search snippets (pages not fetched): Junos https://www.juniper.net/documentation/us/en/software/junos/netconf/topics/task/netconf-configuration-committing-with-confirmation.html ;
IOS-XR https://docs.ansible.com/ansible/latest/plugins/cliconf/iosxr.html ; NETCONF https://www.rfc-editor.org/errata/eid3821 ;
LuCI https://forum.openwrt.org/t/which-daemon-will-do-auto-rollback/114848 ; MikroTik https://help.mikrotik.com/docs/spaces/ROS/pages/328155/Configuration+Management ;
Android A/B https://source.android.com/docs/core/ota/ab ; Kali https://www.cs.toronto.edu/~fanl/papers/kali-issta2015.pdf ;
SWE-bench+ https://arxiv.org/html/2410.06992v2 ; Google https://research.google/pubs/ai-powered-patching-the-future-of-automated-vulnerability-fixes/ ;
CodeMender https://thehackernews.com/2025/10/googles-new-ai-doesnt-just-find.html ; Meta https://arxiv.org/pdf/2507.18755 and
https://engineering.fb.com/2018/09/13/developer-tools/finding-and-fixing-software-bugs-automatically-with-sapfix-and-sapienz/ ;
Sakana incident https://developers.slashdot.org/story/24/08/14/2047250/research-ai-model-unexpectedly-modified-its-own-code-to-extend-runtime ;
Replit https://fortune.com/2025/07/23/ai-coding-tool-replit-wiped-database-called-it-a-catastrophic-failure ; DGM paper https://arxiv.org/abs/2505.22954v2 ;
SWE-bench score aggregators https://benchlm.ai/benchmarks/swe-verified (unverified).
