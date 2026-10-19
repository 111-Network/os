# Layer 6 draft v0.1 — integration, buildability and verification review

Judge: integration lens (fable). Date: 2026-10-19. Read-only; no builds or harness runs (the Layer 5 V4 judge held the box).

## Verdict

**amend-then-lock.** The architecture tracks D25 (wave order is F5 verbatim, the store amendment is additive, the adapter seam is
honest about evidence) and almost every `file:line` is right in today's tree. What is wrong is integration text, not design:
the draft's premise about the agent bearer is false in the built tree, so its "one narrow exception" and its consent story do not
hold as written; §14 rests on an attestation transport no layer has built and no amendment requests; the optional-component seam
as written leaves a default install with no way to obtain the LXMF carrier it puts in the first release; and the supervisor lacks
the ordering primitive §6 and §12 assume. Each is fixable in the spec with one or two paragraphs plus an owner answer. The gates
are mostly runnable on the lead box, but three are not specified enough to write.

## Blockers

**B1. The agent bearer is refused per route, not on writes; the spec builds on the opposite.** Severity: blocker (undermines
D17 #8 and D10 consent, which §15.3 calls structural). Where: §13 "Today the agent's bearer reads and is `401` on writes
(`N/dashboard/api_v1/messages.rs:26-28`)". In the tree `refuse_bearer` (`messages.rs:69`) is called only by messages (send,
retry, cancel), contacts (`contacts.rs:81,189,218`), fabric writes (`fabric.rs:218,295,384,447`) and proposal decisions
(`proposals.rs:609`). `POST /services/{id}/restart` (`services.rs:79`) and `POST /devices/{id}/identify` (`devices.rs:153`)
never call it, so the bearer can restart children and run a "consented" identify today by sending the consent flag itself.
Consequences: the §13 exception already exists unlimited; `device.identify` consent-by-proposal is bypassable; nothing says the
new `/networks`, `/bridge/*`, `/transport/*` and `/messages/fanout` writes refuse the bearer, and G1 asserts it only on edges.
Fix: make the rule global in the spec and the build: a write gate in `require_session` refuses the bearer on every non-GET
except an explicit allowlist (`/services/{id}/restart` for supervised children, rate-limited, logged as an agent event); state
that `/devices/{id}/identify` joins the refused set; G1 asserts `401` on every L6 write and on identify.

**B2. §14 needs an attestation transport that does not exist.** Severity: blocker (unbuildable in R-B without an unlisted
amendment to locked L5 §3.2). Where: §14 "Where attestations travel. In-mesh over the sealed fabric". `grep attest` finds zero
hits in `mesh111` and `node111`; L5 §3.2 has no attestation kind; no store table holds one; `TransportVerifier`
(`ID/attest.rs:298`) has no implementor anywhere. So `attested-111`, "already bridged by node X" (§8 guard 5) and "Open the
sealed thread" (§3) rest on nothing, and §18 lists only "A10 values" under Layer 2. Fix (smallest): keep the A10 byte
assignments and the `TransportVerifier` plug in Wave 0; move gossip, the `attested-111` label, "already bridged" and "Open the
sealed thread" to §17 (second release) and say so in §3, §8, §14. Otherwise add to §18 a new L5 kind, a V3 `attestations`
table, the node111 verifier (Ed25519 for `0x03`, an explicit zero-signature rule for `0x02`) and a G1 assertion.

**B3. The default install cannot obtain the LXMF carrier it ships.** Severity: blocker (scope honesty: Wave 2 and G3 have no
user path). Where: §2.1 rule 2 "an `optional` one also has no payload on disk", rule 3 "release payload or a pre-seeded cache; a
runtime fetch is reserved and not specified", rule 5 "every component starts `undecided` and the default install behaves as
`optional`". Together: no USB drop and no payload in the release means the Reticulum bridge can never be enabled on a stock
install, while §2 cites "the L1 presence-venv pattern", which is an install-time hash-locked `pip download` into
`cache/packages/presence/` (`N/../install/packages.rs:8`; L1 README lines 199-203) and is neither of rule 3's sources. The 47 MB
interpreter tarball per arch makes "in the release payload" unlikely for an optional piece. Fix: adopt the L1 pattern as the
third source ("`111 install` step 5, when online, downloads each declared component's hash-locked payload into
`cache/packages/<id>/`; enable installs from it offline") and amend rule 2 to "no process, listener, loaded code or timer; a cached
payload may sit in `cache/` with its size shown". Needs owner question 1.

## Majors

**M1. No child ordering primitive.** §2 "a child with `after = rnsd`", §6 rule 1, §12 `rns.posture.set` ("stop children,
restart rnsd, restart children"). `ChildSpec` (`N/supervise.rs:37-46`) has no such field and `Supervised::children`
(`services/mod.rs:195`) orders only within one service. §18 Layer 1 lists `ChildSpec.stdio` only. Fix: add the ordering (a
field or a bridge-core orchestration over `ChildControl::stop_and_wait`/`restart`) to §18 and the task cut; G4 asserts the
restart path.

**M2. Foreign state enum and code pins.** §4's machine has `failed`; `MessageState` is closed `[queued, sent, relayed,
delivered]` (`API:687-689`). §10 names no `ForeignMessageState`, omits `unsupported_action` (§12) from the `Problem.code` bump,
and gives no pin list. Fix: declare `ForeignMessageState` as a new closed enum; add `unsupported_action`; list the pins each bump
touches: `proposals.rs:65` `KINDS`, `services/mod.rs:96` `COUNT` plus the four `[_; ServiceId::COUNT]` arrays,
`tests/api_v1_status_services.rs`, the agent count pins, `pnpm gen:types` and `gen-sse.mjs --check`.

**M3. Foreign ingest inside the actor has no batching rule.** The fabric is one loop (`M/fabric.rs:996`) and every typed call is
a synchronous SQLCipher transaction between frame pumps. §6 rule 5 pages the whole MeshChatX backlog on every reconnect and
§5 replays config streams; each row as its own transaction stalls 111 delivery for the burst. Fix: `foreign_ingest` takes a
bounded batch (one transaction per adapter tick), the bridge core awaits each call (back-pressure on the unbounded `Input`
channel), inbound `FabricEvent::Foreign` coalesces per conversation; G1 adds a 1,000-row burst with a 111 latency bound. The
SSE side is fine: `N/fabric/mod.rs:926` coalesces per id and a lagged pump already forces a snapshot.

**M4. Loop-guard protection in prune step 4 is unbounded.** §3 "the origin or copy rows an enabled edge's cursor window still
needs" with no window definition: an edge enabled long ago protects every foreign inbound since, so the only relief is own
history at hard capacity. Fix: the guard needs keys, not bodies; define a bounded `foreign_seen` key set (or "bodies prunable,
ids kept") and the window as N rows per edge.

**M5. G2's stub cannot be the existing one.** `40-interface/verify/stub-meshmud.py` speaks JSON in place of protobuf ("111d
never decodes them"); the §5 client decodes `FromRadio` with prost. Fix: G2 is a `cargo test` in node111 with a test-only Rust
stub on the same prost types; the client treats undecodable frames as counted noise so the L5 harness keeps passing with the
client on; the Nearby parity gate feeds one committed `FromRadio` fixture to both readers (today no recording exists and "one
release cycle" is not a check).

**M6. The gates have no homes.** G7 names the four `check.sh` scripts and `pnpm check`; G1, G3, G4, G5 and G6 are new netns runs
with no script name, flag or location (L5 put G1 in `smoke.sh --three-node` and G3 in `panel-lint.sh`). Fix: name each
(`60-transport/verify/{bridge,lxmf,spine,devices}.sh`, `panel-lint.sh --panel reticulum`) and a `REQUIRE_*` for lock runs. G7's
"idle RSS within noise" is not a criterion; record RSS, assert processes and listeners only.

**M7. Migration leaves two sources of truth.** §11 "mirrored back into `devices.json` while a Layer 5 rollback is supported":
the L5 code reads the binding there (`N/meshtastic/mod.rs:564,802` via `devices/store.rs:256`). Fix: `interfaces.toml` wins; the mirror is write-only; say
what happens when they differ at start.

**M8. App peers are doc-only and cut as normative.** §7 is marked *doc*, `deltachat-rpc-server` over piped stdio and Madmail's
IP-only relay were never run, yet §1 puts them in R-B Wave 2. Fix: a netns spike task before any build row; keep §7 "normative
shape" but mark the Delta Chat adapter as "built after the spike passes" in §16.

## Minors

- `N/panels/manifest.rs:8-9` → the `ServiceId` rule is at `:10-11`; `API:913` → the `kind` enum is at `:919`.
- §2 `PortTable` for loopback listeners: it is the radio-port owner table (`ports.rs:1-15`); the meshmud downlink claim
  (`meshtastic/mod.rs:676`) is the precedent, but only a `bind` detects a foreign holder. Say both.
- §10 proxy: the cookie seams exist with tests (`panels/headers.rs:16-17,127,156`); the proxy wires them. "One server-side
  upstream session" is *(design)*, not "verified at build". Add `X-Forwarded-*` from L5 §13. `DELETE /bridge/edges` needs `{id}`.
- §10 `ServiceId` `reticulum` collides in copy with the `transport` row ("Reticulum"); consider `meshchatx`.
- §11 `[order]` lists both `reticulum` and `lxmf`; define network ids once. `transport.read` spawns Python `rnstatus`: specify a
  tick and cache, never on the request path. D17 #10's per-device region override is absent from the TOML.
- §12 hotplug: libc is already a dependency (`Cargo.toml:36`) and uevent group 1 is unprivileged, but the unit's
  `RestrictAddressFamilies` must keep `AF_NETLINK`. The RNode step needs `venvs/rns`; say the offer appears when any step can run
  and a missing tool is `Error`, never `NoMatch`.
- §3 should say `Class::Foreign` is never produced by `from_kind` (`rows.rs:68-78`) and `is_refusable` stays false.
- §16 G1 "a V3 store refused by a V2 build" is a mesh111 unit test (`user_version = 3` on a V2 schema), not a binary swap; G4's
  "v1 binary leaving a v2 file alone" needs the L5 tag built or a unit test of `existing_schema_version`.
- §5 the presence reader's nonce-0 heartbeat has landed (`meshtastic_presence.py:35`); cite it as built.
- §2.1 component manifests are "parsed at start like `assets/panels`": name the `deny_unknown_fields` rule explicitly.

## Owner questions

1. May `111 install` pre-download hash-locked optional payloads (MeshChatX closure, the 3.11 interpreter) into `cache/packages/`
   when online, so a stock install can enable the Reticulum bridge offline later? Is a cached, uninstalled payload "weighs nothing"?
2. `service.restart`: keep the bearer route (rate-limited, logged, D17 table) or route it as a proposal like every other write?
3. Attestation gossip (a new Layer 5 kind) in the first release, or only the A10 byte assignments now and the labels second?
4. Panel id for MeshChatX: `reticulum` (the draft) or a project name, given the `transport` row already reads "Reticulum"?

## Sound

Wave order equals D25 F5. Schema V3 is an appended migration under `MIGRATIONS` with `SchemaTooNew` refusal
(`schema.rs:46,190-197`); `Class::Foreign = 9` fits `ALL`, `name`, `from_i64`. `local_seq` is shared (`store/mod.rs:517`).
`display-times.foreign` is a second `times.rs` instance. `with_store` is rightly a trap. `MessageId` admits the prefixes.
Routes order rule matches `routes.tsx:40`; the SSE switch has no default; `/settings/l6` and `layers.ts:55` exist; the largest
lazy chunk today is 46.6 KB (jsQR), views 8–14 KB, so a network view fits D21. Tool count 10 → 17 is right
(`daemon_tools.rs:306`, `chat_loop.rs:308`). A10 layout and the opaque `transport_sig` seam match `attest.rs:150-170,298`.
`ChildControl::restart` exists (D16 c); `rnstatus --attach|--detach|--reload` was run (R05 §3.9); migration sources are real
(`store.rs:141`, `rns.rs`); the v1 refusal path is at `transport/mod.rs:149-153,170-171` and `pkill` at `:178`. G3 maps one-to-one
onto R07 checks 1–8. The licensing rows name the sign-off policy in `10-bootstrap/TASKS.md`. Every other cited line was re-read
and is current.

## Proposed task cut

| # | Task | Files | Model | Check |
|---|---|---|---|---|
| T1 | Store V3, `Class::Foreign`, prune 2a/2b, bounded `foreign_seen` | `M/store/{schema,rows,prune}.rs`, `M/store/foreign.rs` | opus | `50-messaging/scripts/check.sh`; migrate, refuse, prune-order tests |
| T2 | Typed `Fabric::foreign_*` with batched ingest, `FabricEvent::Foreign`, daemon publish arm | `M/fabric.rs`, `N/fabric/mod.rs` | opus | mesh111 tests; burst latency test. After T1 (same crate) |
| T3 | Bridge core, `StubAdapter`, edges, fan-out, consent hash | `N/bridge/*` (new) | opus | unit tests: loop guard, cursor, size modes, handoff |
| T4 | Contract: routes, schemas, `ForeignMessageState`, SSE, enum bumps, regen | `API`, `ui/src/lib/api/*.gen.ts` | sonnet | `gen-sse.mjs --check`, `pnpm typecheck`. Before T5, T6 |
| T5 | API routes + global bearer write gate (B1) | `N/dashboard/api_v1/{networks,bridge,fanout}.rs`, `auth.rs`, `devices.rs`, `services.rs` | opus | `tests/api_v1_bridge.rs`; 401 matrix. Needs Q2; serial with T10 on `devices.rs` |
| T6 | Messages rail, `/messages/net/*`, foreign compose gate, `/settings/l6` | `UI/app/routes.tsx`, `UI/views/messages/net/*`, `UI/views/settings/*` | sonnet | `pnpm check` (64 KB chunk), SSE tapes |
| T7 | Meshtastic client via the crate, allowlist, shim disconnect drop, Nearby parity | `N/meshtastic/{client.rs,shim.rs,mod.rs}`, `Cargo.toml`, sign-off row | opus | G2 as `cargo test` with a prost stub. Serial with T10 on `mod.rs` |
| T8 | Spine: `interfaces.toml`, renderer v2, migration, rnstatus attach/reload, `transport.read`, child ordering use | `N/transport/*`, `assets/rns/config.tmpl` | opus | golden renders; `verify/spine.sh` (G4) |
| T9 | Supervisor: `stdio`, `after`, component registry, cache intake | `N/supervise.rs`, `N/components/*`, `assets/components/*`, `install/packages.rs` | opus | unit tests; `verify/zero-weight.sh` (G7). Needs Q1 |
| T10 | Device core: hotplug, identify move + RNode/MeshCore steps, applier | `N/devices/{hotplug.rs,identify/,applier.rs}`, `N/meshtastic/identify.rs` | opus | G5 fixtures (uevent, fake rnodeconf, pty stub). After T7 |
| T11 | LXMF carrier, MeshChatX child, child-panel proxy, `reticulum` panel | `N/bridge/lxmf/*`, `N/panels/{proxy.rs,manifest.rs,listener.rs}`, `assets/panels/*.toml` | opus | `verify/lxmf.sh` (G3), `panel-lint.sh` extension (G6). After T8, T9 |
| T12 | Seven agent tools, pins, untrusted block | `A/src/tools.rs`, `A/tests/*` | sonnet | `30-intelligence/scripts/check.sh` |
| T13 | A10 bytes + `TransportVerifier` plug (scope per Q3) | `ID/attest.rs`, `20-identity/README.md`, node111 verifier | opus | id111 vectors. Needs Q3 |
| T14 | Spike: `deltachat-rpc-server` + Madmail in netns (M8); then docs, LICENSING, KB rows | sidecar scratch; `LICENSING.md`, `00-knowledge/*`, `50-messaging/README.md` §14 | sonnet | spike report; link check |

Not disjoint: T5/T10 (`devices.rs`), T7/T10 (`meshtastic/mod.rs`), T8/T9 (child ordering) — serialize as marked. Owner-gated: T5
(Q2), T9 (Q1), T13 (Q3).

## Method

Read the draft in full against D16, D17, D19–D23, D25, EXECUTION.md, L5 §7.3, §8, §10, §13–§15, §17, §19–§21, L4 §contract,
L1 install and children text, L2 §10 and A10, devices.md §3, §7–§9, research R00, R05, R06, R07 §2 and the options brief §4–§5.
Opened every cited line in `mesh111`, `node111` (store, fabric, supervise, ports, services, panels, transport, devices,
meshtastic, api_v1), `agent111`, `id111`, `openapi.yaml`, the UI files, `check-budget.mjs`, `smoke.sh`, `panel-lint.sh`,
`stub-meshmud.py`, the gate scripts and `Cargo.toml`; grepped for `refuse_bearer`, `attest`, `TransportVerifier`, `after`,
`cache/packages`, `heartbeat`. Measured existing `ui/dist` chunks with `gzip -c | wc -c` (read-only). No cargo, pnpm or harness
was run; nothing left running; no scratch written.
