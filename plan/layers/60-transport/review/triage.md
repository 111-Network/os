# Layer 6 draft v0.1 — lead triage of the three-judge fable panel (2026-10-19)

Panel on `23283ba`: [`bridge.md`](bridge.md) (bridge semantics, relay safety, foreign text) · [`carriers.md`](carriers.md) (adopted
components, process shape, licensing) · [`integration.md`](integration.md) (seams, buildability, verification). **All three: amend-then-lock.**
No finding reverses an owner decision; five findings need the owner (questions at the end, with the lead's recommendation). Everything else
is accepted and goes into draft v0.2 by the spec agent, then one fable re-check (the L5 pattern), then the lock entry D26.

Legend: ✅ accept as proposed · ✅* accept with the lead's variation · ❓ owner question · ✗ rejected (reason given).

## Blockers

| id | finding | disposition |
|---|---|---|
| bridge B1 | relay marker construction and parse disagree: split parts put `(k/n) ` before the marker; a name with `]` or `/` (callsigns) defeats the strict parse → loops with growth across two nodes | ✅ normative marker alphabet (no `]` `/` Cc Cf, UTF-8 boundary, non-empty else `?`), marker first on **every** part, parser accepts an optional `(k/n) ` prefix; G1 adds split-both-ways on two nodes and a `]`-named board |
| carriers B1 = integration B3 | §2.1 rules 2, 3, 5 cannot all hold: an `optional` component with no bytes on disk and no runtime fetch can never be enabled on a stock install; G7 "no payload on disk" passes only where enable cannot work | ❓ **Q1** — lead recommends the L1 pattern: `111 install` pre-downloads each declared component's hash-locked payload into `cache/packages/<id>/` when online (slim closure ≈ 33 MB; CPython 3.11 ≈ 47 MB only when `python3 < 3.11`); "zero weight" = runtime only (no process, listener, loaded code, timer, fd, nothing under `components/` or `venvs/<id>`); cached bytes shown with their size; G7 asserts the runtime baseline and "nothing installed" |
| integration B1 = bridge M4 | the bearer is refused per route, not on writes: `POST /services/{id}/restart` and `POST /devices/{id}/identify` accept the agent's bearer today (verified: `refuse_bearer` only in messages, contacts, fabric, proposals) | ✅ one global rule in the spec and the build: `require_session` refuses the bearer on every non-GET except an explicit allowlist; identify joins the refused set; every §10 write refuses it; G1 asserts `401` on each (the allowlist content is Q2) |
| integration B2 | §14 rests on attestation gossip no layer built (no L5 kind, no table, `TransportVerifier` has no implementor) | ❓ **Q3** — lead recommends: A10 byte assignments + the verifier plug in Wave 0; gossip, the `attested-111` label, "already bridged by node X" and "Open the sealed thread" move to the second release (§17), stated in §3, §8, §14 |

## Majors

| id | finding | disposition |
|---|---|---|
| bridge M1 | `Retry::Later` never wakes (duty-cycle clears with time; no adapter event) | ✅* retry `Later` rows on the monotonic tick with capped back-off (a mechanism, not a policy cap; no wall clock); the 2.1 s pacing timer is the named timed retry (carriers minor 2) |
| bridge M2 | post-air NAKs mapped to terminal `failed`; `relayed` then NAK unspecified | ✅ `failed` only for pre-air refusals; post-air NAKs keep `sent`/`relayed` with the reason ("transmitted, not confirmed"); a DM NAK is `Later` on the next NodeInfo from that node |
| bridge M3 | consent checked once at enable while uplink, licensed mode and the channel key change later; proposal accept skips the warning | ✅ consent facts read live per copy; a changed add-on set pauses the edge with `consent_required` and the new text (lead call: pause, never relay with a flag); `bridge_edge` accept runs the same 409 round-trip; HAM and uplink confirmations use the text-plus-hash record; the uplink switch gates every send on that channel |
| bridge M5 | `service.restart` is a foreign-text → radio-outage path once `bridge.read` exists | ❓ **Q2** — lead recommends: every restart is a proposal (no bearer exception at all; one rule) |
| bridge M6 = integration M4 | no persisted relay high-water; shed copies make history relayable again; "cursor window" undefined and unbounded | ✅ per-edge `relayed_seq` (advanced on shed too); inputs are `cursor_seq < local_seq ≤ … > relayed_seq`; loop guard needs keys, not bodies: bodies prunable, ids kept in a bounded `foreign_seen` set per edge |
| bridge M7 | no shaping quota for `Class::Foreign`; relay backlog starves user sends on air | ✅ `Foreign` gets a shaping quota (evict first, never refuse); the adapter queue takes user-composed rows ahead of relay copies (ordering, not a cap) |
| bridge M8 | proposal bodies render Markdown with links; `bridge_message` carries foreign text verbatim | ✅ `bridge_message` and `bridge_edge` bodies render the text part preformatted; contract says so |
| bridge M9 | stripping stops at Cc (bidi/zero-width are Cf); agent scrubber shape wrong | ✅ one ingest function: strip Cc + Cf (keep `\n` `\t`), NFC, cap; shared by names, bodies, the marker name; the agent uses the `messages.read` scrub shape |
| carriers M1 | `rpc_key` rendered into `rns/config` written 0644 (verified `transport/config.rs:39`) | ✅ renderer v2 writes `rns/config` 0600, `rns/` and the child's client dir 0700; G4 asserts modes |
| carriers M2 | catch-up assumes a global `after_id` endpoint the research shows only per conversation; a store reset strands the cursor | ✅ name the endpoint and verify in G3 (tag *src* until then); reset the cursor on store creation or regression; hash dedupe stays the net |
| carriers M3 = integration M1 | `after = rnsd` is not a supervisor feature (verified: no dependency gate in `supervise.rs`); a double crash partitions silently | ✅ second supervisor extension or bridge-core orchestration: spawn/respawn only while rnsd is `Running` and `rnstatus` shows the shared instance serving; post-spawn partition check with recovery; listed in §18 Layer 1 and the task cut; G4 asserts the restart path |
| carriers M4 | §14 derives Reticulum addressing arithmetic from docs — the derivation pattern §12 rejects | ✅ the attestation carries the LXMF destination hash the child reports over its API; compare, never derive (scope per Q3) |
| carriers M5 | a `native` MeshChatX would redistribute `rns`/`lxmf` wheels against LICENSING §3/§8 | ❓ **Q5** — lead recommends: for any `delivery`, `rns`/`lxmf` wheels come only from PyPI at install time (the rns-lock pattern), never in a release payload |
| carriers M6 | two RNS copies on one node vs the latest-upstream rule after the sunset | ✅* both locks resolve `rns` to the same version at release time; a MeshChatX lock that cannot take the latest RNS is the dated trigger for option C (timing is **Q4**) |
| carriers M7 = bridge minor 13 | Delta Chat consent copy narrower than the core (its own DB holds every contact's plaintext) | ✅ warning once per account; `delete_device_after` for non-bridged chats or the documented residual; the Delta Chat store joins §15.2 and "remove files" |
| carriers M8 = integration M8 | Madmail ports arbitrary, `smp-server` LAN port missing, Delta Chat promised in R-B on doc-only evidence | ✅ ports from the Madmail IP-only doc (standard ports need `CAP_NET_BIND_SERVICE`; Termux cannot), `smp-server` port added; a netns **spike task** before any Delta Chat build row; what R-B ships if the attended gate fails is **Q6** |
| integration M2 | `failed` missing from a closed enum; no pin list; `unsupported_action` missing from `Problem.code` | ✅ new closed enum `ForeignMessageState`; `unsupported_action` added; the pin list per bump (proposals `KINDS`, `ServiceId::COUNT` arrays, status tests, agent count pins, `pnpm gen:types`, `gen-sse.mjs --check`) |
| integration M3 | foreign ingest inside the single actor loop is unbatched | ✅ `foreign_ingest` takes a bounded batch (one transaction per adapter tick), the bridge core awaits each call, inbound `Foreign` events coalesce per conversation; G1 adds a 1,000-row burst with a 111 latency bound |
| integration M5 | G2's stub cannot be the JSON stub-meshmud | ✅ G2 is a node111 `cargo test` with a test-only prost stub; undecodable frames are counted noise; the Nearby parity gate feeds one committed `FromRadio` fixture to both readers |
| integration M6 | gates G1, G3–G6 have no script homes; G7 "RSS within noise" is not a criterion | ✅ `60-transport/verify/{bridge,lxmf,spine,devices}.sh`, `panel-lint.sh --panel meshchatx`, `REQUIRE_*` for lock runs; G7 records RSS, asserts processes and listeners |
| integration M7 | `interfaces.toml` and `devices.json` as two sources of truth | ✅ `interfaces.toml` wins; the mirror is write-only; on divergence at start the TOML wins and the difference is logged |

## Minors

All accepted (bridge 1–15, carriers 1–17, integration list), with these lead readings: bridge 2 (attention) — foreign DMs raise **unread only**, never the
header badge, in v1 (owner may later choose request semantics); bridge 8 — a `pending` row never blocks later rows; packet-id reuse stays forbidden, the
firmware's 20-id window is noted; bridge 9 — one `foreign_messages` row per part (`part k/n`), the target's state aggregated in the UI, cancel drops unsent
parts only; carriers 16 — G6 lints the server-side upstream session (tag *(design)* until built); carriers 17 — the draft keeps G1–G8, R06 references read "R06
G6"; integration `ServiceId` — the MeshChatX panel id is `meshchatx` (the `transport` row already reads "Reticulum"); integration `[order]` — network ids
defined once (`111`, `meshtastic`, `lxmf`, `app:deltachat`, `app:simplex`); `transport.read` runs `rnstatus` on a tick with a cache, never on the request
path; two stale citations fixed (`manifest.rs:10-11`, `API:919`); the presence reader's heartbeat cited as built.

## Owner questions (consolidated; lead recommendation first)

1. **Optional component bytes (carriers B1, integration B3).** Recommend: `111 install` pre-downloads the hash-locked payloads into `cache/packages/`
   when online (≈ 33 MB slim closure; + ≈ 47 MB CPython 3.11 only on hosts with `python3 < 3.11`); nothing installed or running until a human enables
   the bridge; the cached size is shown. Alternatives: USB drop only (no stock path), or decide a runtime fetch later (L1's "no runtime downloads" stands).
2. **`service.restart` for the agent (bridge M5, integration B1).** Recommend: every restart is a proposal, like every other write; no bearer exception.
   Alternative: keep a direct, rate-limited, logged bearer route for non-radio services only.
3. **Attestation gossip (integration B2).** Recommend: A10 byte values + the verifier plug now; the gossip kind, the `attested-111` label, "already
   bridged by node X" and "Open the sealed thread" in the second release. Alternative: build the L5 kind, a V3 table and the verifier in R-B.
4. **Option C timing vs the MeshChatX sunset 2027-01-01 (carriers M6).** Recommend: ship MeshChatX behind the carrier trait in R-B; option C (a sidecar
   importing the user-installed `rns`/`lxmf`, needs a D17 #12 rule change, a LICENSING amendment and a legal read) is triggered when the MeshChatX lock
   cannot take the latest RNS or v5.0.0 lands without a successor. Alternative: decide C now and build it instead.
5. **Natives (carriers M5).** Recommend: `rns`/`lxmf` wheels come only from PyPI at install time for any delivery; which components ship `native` stays
   the owner's hand-selection after the build review (`undecided` in v1). Alternative: a legal read now so a native MeshChatX may carry the wheels.
6. **Delta Chat if the Madmail gate fails (carriers M8, integration M8).** Recommend: a netns spike first; if it fails, Delta Chat leaves R-B (bridge-first
   = Meshtastic + LXMF) and returns with the phone bench. Alternative: ship it online-only with the public chatmail relay as an opt-in.

## Next

Owner answers → D26 (Layer 6 design amendments) → draft v0.2 by the spec agent (every ✅ above + the answers) → one fable re-check on the diff →
lock entry → `os/60-transport/TASKS.md` from the integration judge's task cut (T1–T14, serialization as marked; T5/T9/T13 owner-gated) → build after
the Layer 5 lock.
