# Cross-layer test system: the seam × evidence matrix (D36)

Written 2026-11-11 at `271dabd` from the five survey reports in `research/` (01 aggregate gates and CI, 02 Layer 4 harnesses,
03 Layer 6/7 harnesses, 04 crate and UI suites, 05 the seam inventory), with the facts this matrix leans on re-read in the
sources (nothing built, run or started). Seam ids are 05's. Reads with [`options.md`](options.md).

**Count correction.** 05 says 86 seams; its count (`grep -c '^| [LX][0-9]'`) misses the three `X:` rows. The inventory holds
**89 seams that exist today** (86 `L` rows, `L1-X:pi-floor` among them, plus 3 `X:` rows) and **14 planned Layer 8/9 rows** (103 in all).

## Legend

**Strength** (the best evidence the seam has today):
- **R** running system: a real `111 daemon` (or the real installer) and the real peer on the other side of the seam, in one of
  the namespace harnesses (`4-interface/verify/smoke.sh` single node S1 or `--three-node` S2, `panel-lint.sh`, `6-transport/verify/`
  G1 `bridge.sh`, G3 `lxmf.sh`, G4 `spine.sh`, G5 `devices.sh`, G7 `zero-weight.sh`, `1-bootstrap/verify/linux-e2e.sh --namespace`).
  Every R below is **local only**: no aggregate gate and no CI job runs a namespace harness (`ci-cd.md`; 01 CI-18).
- **I** integration test with the real peer code in one process (`cargo test` with `id111`/`mesh111` path deps, real sockets).
- **F** test against a fake or stub of the other side (sh/py fake children, stub daemons, `stub-*.py`).
- **C** compiled or static only (type generation, lint, digest freshness, a byte budget).
- **N** none.

**When it breaks today** (how far the first failing line is from the seam):
- **A** the failing line names the seam's own interface (a route and its status/body, a file path, a vector file, a rule id).
- **B** a named test function or harness check; the seam must be inferred from the file or test name.
- **C** a stage header or a harness name over a wall of output; or a cascade of unrelated-looking FAILs.
- **D** silent: green, `SKIPPED` with exit 0, or never run.

Harness shorthand: S1/S2 = smoke.sh phases (`5e`, scenario `(8)` …); `n111:<file>` = `1-bootstrap/crates/node111/tests/<file>.rs`;
`m111:`, `id111:`, `a111:` likewise for mesh111, id111, agent111; `ui:<file>` = `4-interface/ui/test/<file>.test.ts`.

## Layer 0

| seam | evidence today | str | when it breaks |
|---|---|---|---|
| L0-L1:kb-payload | S1 `/kb` = `share/kb/llms.txt` over a **hand-built** `payload.manifest` (`smoke.sh` `make_root`); `kb.rs` unit (3); `n111:manifest_layer5` | R | A in S1; B in cargo. The real generator `gen-manifest.py` never feeds a running node |
| L0-L1:index-gate | `build.py --check` (pre-commit, CI); `--selftest` run by nothing | C | A (file + line); can go red by date alone (`STALE_ERROR_DAYS`) |
| L0-L3:kb-grounding | `a111:kb_ritual` (17, fixture KB); S1 5e `kb.read` tool turn via the real agent | R | A in S1; B in cargo |
| L0-L4:kb-api | S1 5b `/kb`, entry traversal; `n111:api_v1_status_services`; `ui:http` | R | A |
| L0-L5/6/7:entries | `build.py --check` (format only; no code reads them) | C | B |

## Layer 1 internal

| seam | evidence today | str | when it breaks |
|---|---|---|---|
| L1-L1:updater-helper | `n111:helper_guard` (18), `n111:updater_stage` (20): temp roots, scripted `systemctl` | F | B; the root path runs only in the V8 VM runbook |
| L1-L1:probation-guard | `helper_guard` f2/f3 (also as local-smoke F2/F3) | I | B; local-smoke's one log `/tmp/smoke-gate.log` is overwritten by the next gate |
| L1-L1:watchdog | none: `watchdog.rs` has 0 tests; `systemd-analyze verify … \|\| true` cannot fail | N | D |
| L1-L1:install-state-tree | `n111:install_flow` (MockRunner); `linux-e2e.sh --namespace` modes 0600 (needs a prebuilt `/tmp/dist-test`; exit 77 counted FAIL by local-smoke) | R | A in linux-e2e (`BAD PERMS: <path>`); as root, permission faults are invisible |
| L1-L1:cli-config | `n111:settings_lan` (3); S1 5d `111 dashboard` prints URLs and the pair code | R | A/B |
| L1-L1:manifest-trust | `n111:manifest_trust` (12), `manifest_parse`, `minisign_verify`, `updater_stage` (real minisign CLI); G12 a T0 stub | I | B |

## Layers 1/2 and 2/x

| seam | evidence today | str | when it breaks |
|---|---|---|---|
| L1-L2:identity-dir | `identity/mod.rs` unit (18, F4 corrupt → quarantine); linux-e2e five identity files 0600; S1 `/status.node.node_id` = generated id | R | A |
| L1-L2:counters-peers-owner | id111 `counters`/`store` contract tests | I | B |
| L1-L2:cli-verbs | `identity/commission.rs` unit (8), `id111` backup/commission tests; no backup → restore through the `111` CLI anywhere | F | B; a CLI-level break is D |
| L2-L4:identity-display | S1 node id; S2 bech32 and label; `n111:dashboard_http` greps responses for key material | R | A |
| L2-L3:signer-none | `intelligence.rs` unit (5) sets `AGENT111_NODE_ID`; `n111:intelligence_children` (sh fake agent) | F | B; nothing checks the real agent stamps `Proposal.node_id` |
| L2-L5:sealer | `m111` `tests/common` real-crypto `Ident` (58 tests); `fabric/sealer.rs` unit (9); `n111:api_v1_messages`; S2 sealed delivery | R | A in S2 (`Message.state`/`reason_code`); B in cargo |
| L2-L5:announce-cards | `m111:hub_flow`, `contacts`; S2 (1) cards via the hub | R | A |
| L2-L5:counters-epoch | S2 (6) `kill -9` → new epoch, (10) clean stop keeps it; `m111:recovery` | R | A |
| L2-L5:store-key | frozen `store-kdf` vectors (`m111` `keys.rs`); `n111:api_v1_messages`; S2 store reopen after `kill -9` | R | B |
| L2-L5:test-vectors | `id111:conformance` (inventory + frozen date), `m111` `frozen_files_match_generation`; `crosscheck.py` ×2 and the Monocypher harness run by **nothing** | I | A (names the vector file) when run; D for the two independent legs |
| L2-L6:attest-bridge | `attest-transport-ids.json` in id111, `bridge/attest.rs` unit, `n111:bridge_lxmf` (`include_str!`); G3 `signed` false → true | R | A |
| L2-L7:epoch-linkkey | none (`relay/linkkey.rs` 10-line stub); G11 `tls.sh` T0 stub | N | D (`G11 SKIPPED`, exit 0) |

## Layer 1 ↔ Layer 3

| seam | evidence today | str | when it breaks |
|---|---|---|---|
| L1-L3:child-spawn | `n111:intelligence_children` (8, sh fake children); S1 real `111-agent` + `stub-llama.py` under real supervision | R | A in S1 (agent log tail printed); B in cargo |
| L1-L3:daemon-token | `n111:dashboard_auth`, `api_v1_messages` bearer matrix; `a111:daemon_tools` stub checks its own bearer; S2 (8) real agent tools; G1 401 matrix | R | A |
| L1-L3:env-precedence | `a111:config_guard` (22), agent side only | F | B |
| L1-L3:health-restart | S1 5g model restart (new pid, old gone); S2 (6) children follow `kill -9` (PDEATHSIG); `probe.rs` unit (11) | R | A; "agent `kill -9` → restart ≤ 10 s" is asserted nowhere |
| L1-L3:offer-hook | `n111:api_v1_status_services`, `install_flow`, `settings_lan`; linux-e2e offer stub | I | B |
| L1-L3:model-manifest | none (status S/P) | N | D |

## Layer 3 ↔ Layer 4 and the agent's tools

| seam | evidence today | str | when it breaks |
|---|---|---|---|
| L3-L4:chat-relay | `n111:api_v1_relay` (9, scripted stub agent); S1 5e real agent; tapes `chat*.sse` replayed by `ui:tapes` | R | A (S1 `FAIL: … [HTTP st: body]`; tapes name tape + frame) |
| L3-L4:agent-events | `n111:api_v1_relay` (`start_events` vs stub); `a111:events_status`; S1 5f waits for `event: agent` | R | A in S1 after a 30 s wait |
| L3-L4:proposals-fs | S1 5f (a hand-written proposal + one from a real `diagnose` task); `api_v1_relay`; UI proposal views on the mock | R | B; the agent's kinds vs daemon `KINDS[9]` vs the yaml enum are pinned by hand on each side |
| L3-L4:task-route | `api_v1_relay` `needs_hub`; `a111:task_engine` (fake daemon); S1 5f `POST /agent/task` 202 | R | A |
| L3-L4:prompts-tools | `ui:prompts` on the mock; no check that a prompt's tool exists in `tools.rs` | N | D (agent answers "I cannot") |
| L3-L5:daemon-tools | `a111:daemon_tools` (15) on a hand-typed stub `111d` never validated against the contract; S2 (8) real agent → real daemon | R | A in S2; agent side stays green when a contract field is renamed |
| L3-L6:l6-tools | `a111:layer6_tools` (20, second stub-daemon copy); node111 proposal tests; G5 decides harness-written `device` proposals | F | B; no real agent ever calls an L6 tool |
| L3-L7:l7-tools | none (row A1 ⬜) | N | D |
| L3-L7:offload | `a111:config_guard` private-IP refusal; registry is a stub | F | B |

## Layer 1 ↔ Layer 4

| seam | evidence today | str | when it breaks |
|---|---|---|---|
| L1-L4:legacy-contract | `n111:dashboard_http` (4), `api_v1_status_services`; S1 `/api/status` schema 1, `/health` | R | A |
| L1-L4:ui-embed | S1 5a `GET /` = `dist/index.html` sha256; `install/verify.rs` unit; `cross-build-ui-gate.sh` | R | A in S1; `check.sh` runs `cargo test` **before** the UI build, so Rust tests see the "not built" fallback |
| L1-L4:bind-host | S1 5a Host guard; `dashboard_auth` Host matrix; `settings_lan` | R | A (`:80` only a `note:` when it does not bind) |
| L1-L4:pairing-auth | S1 5d from a real non-loopback source; `dashboard_auth` (17); panel-lint `lan` pairs through the form | R | A |
| L1-L4:restart-update | S1 5g/5h (exit 0, no child left); dashboard tests | R | A |
| L1-L4:services-registry | S1 `/services` = 9 ids; `api_v1_status_services` (pins by hand, reads no digest); `ui:gen` `SERVICE_IDS` vs yaml | R | A; Rust `ServiceId::COUNT = 9` vs yaml has no automatic check |
| L1-L4:devices-api | `n111:api_v1_devices` (15; the root case returns early and counts PASS); G5; S1 `/devices` | R | A in G5; D for the root case |
| L1-L4:events-bus | S1 5c `reset` + `Last-Event-ID` replay + restart reset; `dashboard_auth`; tapes | R | A |
| L1-L4:install-print | `install/verify.rs` unit (8) | F | B |

## Layer 4 contract seams

| seam | evidence today | str | when it breaks |
|---|---|---|---|
| L4-L4:openapi-ts | `ui:gen` (`gen-sse.mjs --check`; the `types.gen.ts` compare **returns silently** without the binary); CI `gen:types` + `git diff` of `src/lib/api` | C | A (`stale: <file>`) |
| L1-L4:openapi-rust | five node111 files validate against `ui/test/contract.gen.ts` (each with its own validator port); `dashboard_auth` walks every operation (`UNROUTED` 9); no route-set diff | I | B (`assert_schema` panic); a stale digest is read as truth on a Rust-only run: D |
| L3-L4:tool-count-pin | `a111:daemon_tools`, `chat_loop` pin 17; the API README pin list is prose | C | B |
| L4-L1:tapes | `ui:tapes` replays 11 recorded tapes; re-record only with `SMOKE_TAPES=1` in a namespace run | R | A (tape + frame); stale tapes stay green: D for drift |
| L4-L4:budget | `check-budget.mjs` in `pnpm check` | C | A (names the chunk, not the module) |

## Layers 1/4 ↔ Layer 5

| seam | evidence today | str | when it breaks |
|---|---|---|---|
| L1-L5:fabric-start | `n111:api_v1_messages` (real mesh111, SQLCipher, tcp-lan); S2 `GET /fabric` per node, LAN listeners follow settings | R | A |
| L4-L5:messages-api | `api_v1_messages` (12, every §15 route vs the digest); S2 (2) (4) (7) states and reason codes | R | A; S2 chains: one early FAIL cascades |
| L4-L5:sse-fanout | S2 SSE `new` → `delivered`, (15) frames; tapes `events-l5-*`; `api_v1_messages` frames; G1 1,000-row burst = one frame | R | A |
| L1-L5:fabric-cli | S2 phase 6 `111 fabric offer`/`enroll`, spent code 502 | R | A |
| L4-L5:panels | panel-lint 3b curl matrix + 3c browser rules (`<scope>/<rule-id>`); S2 (15); `n111:api_v1_panels` (19) | R | A (the only harness with stable rule ids); RED by default on three `blocked-by P1 guard.rs` gaps |
| L1-L5:sidecar-children | S2 (15), panel-lint 3a: **stub** meshmud and presence supervised by the real daemon; `daemon_presence_venv`, `supervise_pdeathsig` | F | A; the real `meshmud` binary runs in no harness |
| L5-L6:presence-file | `stub-meshmud.py --role presence` writes `meshtastic.json` → S2 `/nearby` 3 nodes; the real sidecar's 97 tests check its own output; `presence.rs` unit (5) | F | B; no shared schema between the real reader and the daemon |
| L1-L5:payload-assets | `n111:manifest_layer5` (16); `build-scripts-gate.sh` | I | A |
| L1-L5:blob-intake | none (planned) | N | D |

## Layers 5/6 ↔ Layer 6

| seam | evidence today | str | when it breaks |
|---|---|---|---|
| L5-L6:foreign-tables | `n111:bridge_core` (32), `api_v1_bridge` (22); `m111:foreign_burst`; G1 (a V3 store refused by the Layer 5 tag's V2) | R | A |
| L5-L6:link-trait | `m111:lossy`, `links` (test link); `kiss-modem`, `ble` unbuilt | I | B |
| L5-L6:meshmud-client | `n111:meshtastic_client` (27, real prost framing); G3 with `lxmf/fake-board.py`; panel-lint shim round trip | F | B; three fakes of one peer (two `stub-meshmud.py` with JSON payloads, one hand-encoded protobuf board) |
| L4-L6:networks-bridge-api | `api_v1_bridge` vs the digest; G1 consent, fan-out, `oversize`; `ui:layer6-api`, `layer6-contract` (hand fixtures) | R | A |
| L1-L6:rnsd-child | G3, G4 real rnsd 1.5.7; `n111:transport_spine` goldens (21); `transport_venv` stub rnsd | R | A; G3/G4 SKIP silently without the package index |
| L4-L6:transport-api | `n111:api_v1_transport` (fake `rnstatus`); G4 posture, QR join | R | A |
| L1-L6:devices-hotplug | G5 (fake `/dev`, `/sys`, uevents); `n111:devices_core` (26) | R | A; G5 needs a mount namespace and a 100 MB download |
| L1-L6:component-cache | `n111:components` (9); G3 enable → disable → remove leaves nothing | R | A |
| L6-L6:lxmf-sidecar | G3 real helper + real rnsd; `n111:bridge_lxmf` on `fake_lxmf_sidecar.py`; the helper's 128 tests on a fake RNS (CI) | R | A in G3; the cargo fake shares only README text with the helper |
| L6-L6:deltachat-madmail | `n111:bridge_deltachat` (17) on `fake_deltachat_rpc.py` (admits verbs the real core never ran) | F | B |
| L6-L6:simplex | `bridge/simplex.rs` unit (1) | F | B; nothing runs `simplex-chat` |
| L1-L6:bridge-stubs | used by G1; "a release build compiles none of it" is checked by nothing | C | D for the release claim |

## Layer 7

| seam | evidence today | str | when it breaks |
|---|---|---|---|
| L5-L7:pair-gate | `m111:forward` (26, real tcp-lan on loopback); daemon side partly landed (M5); G9 a T0 stub | I | B (a ~3 min suite) |
| L5-L7:forwarder-handoff | `m111:forward`; `carrier/mod.rs` 0 tests; G9/G10 stubs | I | B on the mesh side; D on the daemon side |
| L5-L7:custody-store | `m111` `store::` lib (81), `forward`; S2 13b quota → `mailbox_full` (member rows only) | I | B |
| L5-L7:fabric-tap | none (S) | N | D |
| L5-L7:role-bits | `m111` hub/forward tests for bits 0–2; bits 3–5 unbuilt; `ui:layer7-contract` | I | B |
| L6-L7:netlink-gateway | `netlink.rs` unit (2); D3/D4 ⬜; G7-extended `idle.sh` a stub | N | D |
| L6-L7:hops | none (planned) | N | D |
| L1-L7:origin | client side: `updater_stage`, `helper_guard` (L2, L3 landed); server `origin/mod.rs` 11-line stub; G12 stub | I | B client; D server |
| L1-L7:relay-state | `relay/settings.rs` unit (19, D1 landed); `/relay` routes 404 until D10 | I | B |
| L4-L7:relay-api | `ui:layer7-contract` (37, hand fixtures "not daemon output"); 8 L7 ops in `dashboard_auth` `UNROUTED` | C | B |
| L1-L7:carrier-ntfy | `n111:carrier_ntfy` (11) vs `stub-ntfy.py`; `carrier.sh --smoke` (G10-smoke); the stub is unverified against a real server | F | A |
| L1-L7:registry-dnssd | none (S) | N | D |
| L1-X:pi-floor | G7 `zero-weight.sh` (needs a live daemon; no harness calls it; SKIPPED otherwise); S1 no child left after restart; G3 hygiene; UI budget | R | D by default (`G7 SKIPPED`); A when hand-run |

## Cross-cutting

| seam | evidence today | str | when it breaks |
|---|---|---|---|
| X:licensing-notices | `gen-notices.py --check` + two unittest suites (check.sh, CI) | C | A (diff) |
| X:manifest-pins | `manifest_*` tests, `cross-build-asserts-gate.sh`, linux-e2e | I | A |
| X:plan-vs-code | pre-commit `build.py`; CI one-digit-folder grep; `file:line` citations in boards checked by nothing | C | D |

## Planned (Layers 8 and 9)

L3-L8:agent-state · L1-L8:state-service · L8-L9:heal-store · L5-L8:tier2-objects · L2-L8:dataset-signing · L7-L8:move-datasets ·
L4-L8:hub-content · L1-L8:updates-backup · L8-L8:shared-log · L1-L9:observer-change-engine · L3-L9:diagnosis-playbooks ·
L2-L9:fix-signing · L5/7-L9:fix-sharing-delegation · L0-L9:evolution: no code, **N / D** by construction. They enter the seam map
as `planned` rows now so each spec's seam section reuses the ids (options.md F5).

## Totals

| strength | seams | of which in CI |
|---|---|---|
| R running system | 42 | 0 (only `ui:tapes` replays a recording of one) |
| I integration with the real peer | 15 | all, through the cargo suites (bootstrap-gate, identity, intelligence jobs) |
| F fake of the other side | 13 | all, through the cargo suites |
| C compiled / static | 9 | all except the `bridge-stubs` release claim and plan-vs-code citations |
| N none | 10 | — |

Two of the 42 R rows (`L1-L1:install-state-tree`, `L1-X:pi-floor`) need a hand step first (a prebuilt musl binary; a live daemon).

## Gaps

**1. Seams with no running-system evidence (47 of 89).** Built and without R: `L1-L1:updater-helper`, `probation-guard`, `watchdog`,
`manifest-trust`; `L1-L2:counters-peers-owner`, `cli-verbs`; `L2-L3:signer-none`; `L2-L5:test-vectors`; `L1-L3:env-precedence`,
`offer-hook`; `L3-L6:l6-tools`; `L3-L7:offload`; `L1-L4:install-print`; `L1-L4:openapi-rust`; `L1-L5:sidecar-children` (stub
children); `L5-L6:presence-file`, `meshmud-client`, `link-trait`; `L6-L6:deltachat-madmail`, `simplex`; `L1-L5:payload-assets`; every
Layer 7 seam. Of these the cheapest real-to-real wins are `L3-L6:l6-tools` (a real agent call in G1's two-node run) and
`L5-L6:presence-file` (the real reader in S2 instead of the stub role).

**2. Fakes not checked against what they mirror.**
- `a111` stub daemons ×3 (`daemon_tools.rs:110`, `layer6_tools.rs:169`, `task_engine.rs:72`): hand-typed JSON, never validated against
  `contract.gen.ts`; a renamed contract field leaves agent111 green.
- node111's stub agents (`intelligence_children.rs` sh scripts, `api_v1_relay.rs` scripted agent) vs the real `/v1/chat` and `/v1/agent/*`.
- `fake_lxmf_sidecar.py` vs `lxmf_sidecar.py` (they share only the README); `fake_deltachat_rpc.py` vs the real core.
- `stub-meshmud.py` ×2 (`4-interface/verify/`, `6-transport/verify/devices/`) and `lxmf/fake-board.py` vs meshmud and the presence reader.
- `stub-ntfy.py` vs a real ntfy server (11 log items "not run from this box"); `stub-llama.py` vs llama-server.
- The UI mock (4,762 lines) is held to the contract, not to daemon output.

**3. Fixtures copied by hand across a layer boundary.**
- The JSON-Schema-subset validator: `ui/test/helpers/validate.ts` plus five Rust ports (one per node111 test file).
- `smoke.sh` `make_root` writes `payload.manifest` and the KB tree by hand instead of with `gen-manifest.py` (L0-L1).
- Proposal bodies (`bridge_message`, `bridge_edge`, `device`) pinned byte-exact separately in agent111 and node111 tests.
- Agent proposal kinds vs `KINDS[9]` vs the yaml enum; `ServiceId::COUNT` vs the yaml; the tool count 17 in three places.
- Three scratch-root builders: `smoke.sh make_root`, `6-transport/verify/root.sh g6_make_root`, `spine/node.sh g45_make_root`.
- Recorded and unread: `6-transport/verify/tapes/events-l6-*.sse` (no consumer; `tapes.test.ts` reads only `ui/test/tapes`).

**4. Harnesses whose failure names no assertion.**
- `1-bootstrap/scripts/check.sh` stops at the first failure and names only the last `== stage ==` header; the nested
  `5-messaging/scripts/check.sh` prints its own green line mid-stream; `3-intelligence/scripts/check.sh` prints no stage header at all.
- `smoke.sh` and `panel-lint.sh`: a `set -e` abort inside the namespace prints no `FAIL:`, the EXIT trap still writes `FAILS=0`,
  and the outer prints GREEN (`smoke.sh:1653-1657`, `panel-lint.sh:566-569`; found by reading, not run).
- `local-smoke.sh`: every gate overwrites `/tmp/smoke-gate.log`, so the path in a FAIL line shows the next gate's log.
- cargo, vitest and pytest stages: a named test inside a wall of output; no seam tag, no log path.

**5. Checks that silently skip or pass.**
- `check.sh`, `5-messaging/scripts/check.sh`: every skip is exit 0; the difference lives in the last line only.
- `6-transport/verify/lib.sh skip_or_fail` exits 0 `SKIPPED` after `FAIL:` lines were already printed (`spine.sh:557`,
  `devices.sh`, `bridge.sh:1097` after a failed build): the exit code lies.
- G3/G4/G5 SKIP without the package index; G7 SKIPs without a live daemon; G7-ext, G8, G9, G10, G11, G12 print `SKIPPED (T0 stub)`.
- `api_v1_devices.rs` root case returns early and counts PASS; `gen.test.ts` returns when `openapi-typescript` is absent;
  the presence sidecar's real-library test skips invisibly under `-q`; `carrier.sh --smoke` passes "no longer listens" when `ss` is absent.
- As root, 8 node111 tests fail by design (`6-transport/TASKS.md:80`) and only a board sentence says so.
- `build.py --check` turns red by date alone (`STALE_ERROR_DAYS` 180).

**6. Not in CI.** Every namespace harness (S1, S2, panel-lint, G1, G3, G4, G5, G7, linux-e2e) and therefore every R row; musl/G2;
the Monocypher harness; both `crosscheck.py`; `build.py --selftest`; `shim/tests/run.sh`, `termux/tests/dryrun.sh`, `local-smoke.sh`;
shellcheck of `3-intelligence`, `4-interface/verify`, `6-transport/verify`, `7-relay/verify` and `hooks/`. CI writes no
machine-readable result (no artifact, no annotation beyond two `::error::` lines).
