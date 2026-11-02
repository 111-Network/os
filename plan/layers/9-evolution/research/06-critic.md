# Layer 9 research 06: completeness critique of 00 to 05

Critic: opus, 2026-11-02. Read all six reports, the cited lines and Layer 8's sweep; six web checks; nothing run.

## 1. Verdict

The sweep is enough for an options brief on the core: event-driven observation in 111d, incidents, deterministic playbooks, the typed
apply-verify-undo engine, consent and the Repairs page. Gap-fill is needed in one place, the hub code-fix path, rated **blocking**: as built it is
unsafe (§2.1). 03 and 05 treat it as settled; 00 flagged it unverified. One hand-off was missed (`AGENT_OP`). Twelve conflicts are for the brief, not for research.

## 2. Settled by the critic

1. **opencode runs with every tool allowed, bash included.** Upstream: "By default, all tools are **enabled** and don't need permission to run"
   (`docs/tools.mdx:8`; "Most permissions default to `allow`", `permissions.mdx:174-176`); `--pure` only drops external plugins (`cli.mdx:743`); the
   tools include `bash`, `webfetch`, `websearch` (`tools.mdx:48,238,255`). The agent's `opencode.json` has no `permission` block
   (`agent111/src/tasks.rs:495-508`). Layer 3's "no host shell for the agent" (`3-intelligence/README.md:55`) does not hold on hubs, and lens 0's
   trap 4 (loopback is the human) is real as read, untested.
2. **P4 vs D26 is narrower than reported.** D26 #2 ("Agent restarts — every restart is a proposal … the agent's bearer is refused on every write",
   `plan/DECISIONS.md:392`) governs the agent's bearer, not a restart 111d makes inside a user-set scope, which P4 (`parked.md:11`) and
   `cross-layer/devices.md:194` ("rate-limited 1/5 min/service") allowed. Still the owner's call (fork 1); no locked text forbids it.
3. **git GPL-2.0-only and llama.cpp MIT** were read by lens 5, clearing lenses 3 and 4. 4. **The Pi floor includes 111-agent** (5.3 MB,
   `6-transport/TASKS.md:82`); lens 0's "without 111-agent (the Pi floor)" means Cat 1 or a disabled agent.

## 3. Coverage against README §9, the principles and the hand-offs

Covered: every §9 clause (00-04), no telemetry, the device floor, the L1, L3-L7 and P4 hand-offs. Thin or missed:

- README:20: the agent "sends updates and PRs"; no scout cites it, all pick a human sender (fork 6).
- Cat 2A/3 "consume … through a nearby hub" (README:118): only LAN offload is covered (05 F6). `AGENT_OP` (`5-messaging/README.md:173`, a Layer 9
  hand-off at `:832`) appears in no report (G2).
- L1 "APK refresh = Layer 9 maintenance task" (`1-bootstrap/README.md:160`): missed (minor; a human-instruction proposal, no adb on a phone).
- "No rate limits" (README:79) vs the proposed caps (auto-apply 3/day, per-author hourly, per-hub daily PR): only 05 Q7 (fork 8).

## 4. Conflicts the brief must resolve

1. **Engine shape.** 00: an `Action` trait and `repair` kind now. 02: a trait "only when a fifth action family arrives", yet 02 adds settings
   PATCH and overlays and 04 adds `bridge.*`: it has arrived.
2. **What auto-applies in v1.** 02: only the undo (T2 specified, off). 05: an opt-in rung, cap 3, `service.restart` in scope.
3. **After the keep.** 02 F9: a new proposal. 00, 05: a post-keep probation. 05 keeps 5 snapshots; 02 one armed `undo.json`.
4. **Prompt and playbook overlays.** 02: tier A, auto-eligible under a grant. 04: no node overlay in v1; auto-apply of data breaks rule 1.
5. **Playbook home and grammar.** 00: `repair-<service>-<reason_code>` recipes; 01: `skills/` plus a YAML mini-language; 04: a Layer 0 `playbook`
   category with SKILL.md fields; 02: a node overlay. 01 runs checks "by 111-agent code", yet T0 must work model-less and the observer is in 111d.
6. **Local model on 2B.** 01 T1 uses qwen3-4b on the node; 05 only on 4A/4B; Layer 3 gives 2B qwen3-4b (`3-intelligence/README.md:33`).
7. **Code validation.** §9: validate, then the PR. 03: "validated" = upstream CI green, after the PR. 02 C3: an opt-in pre-PR hub sandbox.
8. **Incoming fix text.** 03 §4.3: "never read by the agent" and "the receiving agent's job is to summarise". Summarising is an injection path.
9. **Exchange and storage.** 03: v1 `PATCH` to contacts or own hub (contacts or hub-vouched). 05: receive from the home hub only, and code pulled
   "`111 update --from`-style", though the hub origin serves only release manifests and pinned blobs (00 trap 12). Layer 8 puts Layer 9 state in a
   `heal/journal` namespace (`8-storage/research/04-local-store.md:91-95`) and floats a p2panda endorsement feed (`03-shared-log.md:66-68`); Layer 9
   picks JSONL files. 02's capacity-pruned audit and 05's hash chain need a checkpoint rule.
10. **Wakeup baseline.** 00: ≈265/min; 01: ≈100-120/min; both arithmetic; 01's "+0.06 %" for option B rests on it.
11. **Domain tags.** `111/proposal/v1`, `111/patch/v1` (00) vs `111/fix/v1`, `111/fix-vouch/v1` (03): one set, in L2 §6.
12. **Android cron.** 01 reuses it for option B; 05 replaces it with a `finish`-hook inbox.

## 5. Gaps

Unsafe yet settled: opencode as-is (03, 05), auto-eligible overlays (02), agent summaries of foreign text (03). Custom despite an adoptable
project: G3. Periodic without a number: G6.

| # | Gap | Rating |
|---|---|---|
| G1 | **Hub code-fix pipeline.** Tools verified permissive (§2.1). Open: loopback-as-human; `webfetch` on a gateway hub; a runtime npm provider install via embedded Bun (search snippet; rule 5); no source tree at the running release; offline workspace build (vendored crates, toolchain, RAM, time on 4A/4B); a sandbox (bwrap, or netns without loopback). Decides whether code fixes exist in v1. | **blocking** |
| G2 | **`AGENT_OP` and delegation for 2A/3 over the fabric.** Today only LAN offload is covered (L3 v1). | major |
| G3 | **Adopt vs build for playbook rules.** 01 invents an `expect:` mini-language; alternatives: `cel` 0.15.0 (MIT, LICENSE read, 2026-10-06), `serde_json_path` (RFC 9535, MIT), or a closed predicate enum. Missed prior art: StackStorm rules (Apache-2.0, read), Salt reactor, Nagios/Icinga event handlers, MAPE-K. | major |
| G4 | **A joint storage contract with Layer 8** (conflict 9). | major |
| G5 | **Wakeup baseline measured, not counted** (conflict 10). The lead does it on this box, in a G7-style netns run. | minor |
| G6 | **Unquantified items.** Post-keep window length and mechanism (re-probing is a poll); per-incident debounce; qwen3-4b reload per incident after `--sleep-idle-seconds`; 03's "accepted_by_n … for a stated period" (needs a clock-free definition). | minor |
| G7 | **Constrained decoding.** Format restrictions hurt reasoning, can help classification (Tam et al., arXiv 2408.02442, search snippet): keep T1 a pure classification, narrate separately. | minor |
| G8 | **No v1 cut sized across the lenses.** The union amends L0, L1, L2, L3, L4 and L6, most of them locked; the lead phases it. | minor |

## 6. Owner forks

1. v1 autonomy: propose plus automatic undo, or an opt-in auto-apply rung for typed reversible actions in a user scope (incl. 111d restarting a `Degraded` child).
2. Observation: events only, or plus an opt-in 15-min deterministic probe.
3. Diagnosis on a 2B Pi: local qwen3-4b, or playbooks plus hub only.
4. Playbook home; agent data release-only or a node overlay later.
5. Code fixes in v1: none, or hub-drafted under G1's containment; validated pre-PR on a hub or by CI after.
6. PR sender: the human (mbox + PR.md) or an opt-in hub token (README:20).
7. Peer fixes in v1, trust (contacts or hub-vouched) and receive/send defaults.
8. Default caps vs "no rate limits" (README:79).
9. Signing: root key with new tags now, or the L2 delegation cert.
10. DCO sign-off and a CI check.
11. KB drafts export-only; widen the bridging vocabulary (reverses `6-transport/README.md:458-459`).
12. Replace the Android cron; Layer 9 state in own files or a Layer 8 namespace (joint with Layer 8).

## 7. Unverified claims, for the lead to check on the owner's machine

- **Licences:** systemd repo-wide; crates `inotify`, `sd-notify`, `landlock` (crates.io metadata only); rpcd (header only); Termux:API,
  termux-boot, termux-services; cronie; Cockpit; "or later" for ABRT, snapper, ostree; Unsloth; OpenHands `enterprise/`; SICA; DEP-3.
- **Evaluations (all search snippets):** ITBench 11.4 % (v1 13.8 %), OpenRCA 11.34 %, OpenRCA 2.0 20.7 %, RCACopilot 0.766, log anomaly 98.9/82.4 F1,
  Huang, Self-Refine, SICA 17→53 %, ICSE 2026 7.8/29.6 %, SWE-ABS 19.78 %, SWE-bench+ 32.67/31.08 % (versions differ), Kali 2/105, Google 15 %,
  Meta 25.5/80 %, CodeMender 72, Sclar 76 pts, ACE, MINJA, AgentPoison, MemoryGraft, Shumailov, GEPA, METR 1-2 %, Tam.
- **Incidents and policy:** DGM fabricated log and removed markers; AI Scientist timeout; Replit deletion (07-18 and 07-23 both cited);
  Invariant Labs MCP leak; curl bounty end 2026-01-31; the Copyright Office report.
- **Weights and speeds:** qwen3-4b on a Pi 4/5 (2.3-5 tok/s, prompt eval, reload); the 20 min diagnose estimate; ≈265 vs ≈110 wakeups/min;
  RSS of Netdata, monit, Prometheus, opencode, Cockpit; incident record size; eval-budget and 64 GB fine-tune arithmetic.
- **Platform:** NETLINK_KOBJECT_UEVENT and inotify under Termux; CONFIG_PSI on Pi OS; `power_supply` capacity uevents; Android 15 wake-lock kills;
  the Termux:API job-scheduler minimum; bwrap and Landlock on Android; any kernel free-space event.
- **Vendors:** GitHub secondary limits, fine-grained PAT fork PRs, URL cap, fork-workflow approval; Dependabot/Renovate limits; Promptfoo
  acquisition; Agent Skills adoption; Junos/NETCONF/IOS-XR/LuCI/MikroTik defaults; opencode skills and runtime provider install; in-toto CNCF.
- **Repo:** `build.py` inside `share/kb/`; agent111 test counts; `proposals.rs:412-475` (carried from 00).

## 8. Sources (critic's fetches, 2026-11-02)

https://raw.githubusercontent.com/anomalyco/opencode/dev/packages/web/src/content/docs/{tools,permissions,cli,providers}.mdx ·
https://crates.io/api/v1/crates/{cel,serde_json_path} · https://raw.githubusercontent.com/{cel-rust/cel-rust,StackStorm/st2}/master/LICENSE ·
search snippets: https://arxiv.org/abs/2408.02442, https://bun.com/docs/runtime/auto-install.md. Repo lines are cited inline.
