# Layer 7 draft v0.1 — lead triage of the three-judge panel (2026-11-02)

Panel on `30ffc1d` (run `wf_27c6c677-781`, three fable judges, read-only): [`custody.md`](custody.md) (C, 18 findings) ·
[`carriers.md`](carriers.md) (S, 27) · [`integration.md`](integration.md) (I, 16). **All three: amend-then-lock.** Four blockers
(C1, C2, S1, I1), seventeen majors, forty minors; beside them the drafter's eleven deviations D1–D11 and its three questions. No
finding reverses an owner decision (D30). Six items touch the owner (Q1–Q6 at the end); each is applied in v0.2 as the lead
recommends and flips in v0.3 if the owner says otherwise. Everything else is accepted and goes into v0.2 by the spec editor
(`runs/workflows/spec-fix.js`), then one fable re-check (`recheck.md`), then the lock entries (D31 amendments, D32 lock).

Legend: ✅ accept as proposed · ✅* accept with the lead's variation (the variation wins) · ❓ owner question, applied as the lead
recommends · ✗ rejected (reason given). The finding texts are in the three reports; this file carries only the dispositions.

## Blockers

| id | disposition |
|---|---|
| C1 | ✅ all three parts: (a) a forward row every reachable target refused is marked `refused` (committed = 1, step-1 evictable, a `refused` count in `FabricStatus` and the fabric view; the member keeps `relayed` and §5.6 states the honest limit); (b) a per-origin share inside Transit, `quota.transit / 8` per member and per pair, over which that origin's puts get `REJECT_FULL` while others are admitted; (c) §5.8 and §14 say protected Transit rows count toward the high watermark. G9 gains the refused-row and second-origin assertions. The acknowledgement the judge asks for is Q6 (within F4; the lead sees no new decision). |
| C2 | ✅ the no-hint clause rewritten: a target exists (any tcp or gateway pair; or a carrier pair and the put fits) ⇒ hold `fwd = 1`, answer `ACCEPTED`, probe per §5.5 event-driven after the ack; no target at all ⇒ `REJECT_UNKNOWN_DEST`; the hinted-but-unmatched clause matches; §5.6 says the first hub's `ACCEPTED` means "held and will be offered", never "the dest's hub was found". |
| S1 | ✅ specified as new: `state/staging.lock` (flock, O_CLOEXEC) taken by `check_once` for the whole fetch–stage–request sequence in the daemon loop and in `111 update --from`; a held lock makes the verb print "the daemon is checking now" and exit 75; G12 asserts a concurrent run stages once. |
| I1 | ❓ Q2 → (b): a read-only `Fabric::attributed_on(peer) -> Option<LinkKind>` (or a `via` on the Message view: the editor picks the smaller), recorded in §19 as a code seam beside D1 (outside the twelve, read-only, byte-identical); §2.2 row 1 names the rule; G8 asserts the LAN-vs-hub split. |

## Majors

| id | disposition |
|---|---|
| C3 | ✅ the bundle carries the offering side's proposed role; the scanning side's pick must be the complement or `POST /relay/pair` answers `409 pair_role_mismatch`; a carrier pair is `peer` only; a leaf pair is tcp only; a forward from a pair whose recorded role conflicts with the row's `fwd` is refused `REJECT_UNKNOWN_DEST`; G9 adds the asymmetric case. |
| C4 | ❓ Q3 → the v1 limitation stated in §5.1 and §17: a hub's own messages reach other islands only through its own home hub; an own-send route into its own forward table is v1.1 beside the own-send carrier route (D30 Q20). |
| C5 | ✅ the whole scope sentence replaced as the judge words it, with the `queued` (hub unreachable) / `sent` (`REJECT_UNKNOWN_DEST`) reading that matches the code. |
| C6 | ✅ a posted row is re-posted only when a drain returned a frame from the far hub newer than the post and still no ack, or on Flush now, or on a since-miss after a server restart; reposts per pair counted in `RelayStatus`; G10 asserts one post per row per far-hub round. |
| C7 | ✅ acks are consumed on take and never retried; a lost ack heals because the far hub re-posts the put (C6) and the next `carrier_inbound` answers `DUPLICATE`, a fresh ack; `flush` returns `Vec<(Option<u64>, ForwardDone)>`. |
| C8 | ✅ amendment 6 names `prune.rs:484-509` and `:228-245`: `class_bytes(Custody)` = `WHERE fwd = 0`, `class_bytes(Transit)` = `WHERE fwd != 0`, `evictable` assigns `Class::Transit` to `fwd != 0` rows; `is_refusable` and `has_quota` include Transit; `custody_put` takes `fwd` and admits under the row's class. |
| C9 | ✅ one record: mesh111 `PairedHub {node_id, role: Peer | Leaf | Gateway, via: Tcp | Carrier}` in `FabricSettings` (persisted through `FabricEvent::Settings`); the daemon keeps address, SPKI, lan and the warning hash beside it; §4.6's JSON and §5.3 row 10 match. |
| S2 | ✅ the pinned procedure (`set_ip_check_interval(1)`, one one-shot sleep of 1,500 ms, `set_ip_check_interval(0)`, in the one task that owns the mdns handle); G8 asserts the interval reads 0 within 2 s of every event and the A records changed; the unregister-and-re-register form is a build bench item. |
| S3 | ✅ the interval is set to 0 only after the second netlink socket is bound; on a bind failure mdns-sd's default 5 s check stays and is logged once; §16's −12/min reads "where the bind succeeds". |
| S4 | ✅ `origin_base = http://<home_hub.address host>:11180/origin/v1/` (DEFAULT_PORT; `:80` never assumed); the hub serves on every LAN-bound dashboard listener; the Host guard admits the hub's own addresses, stated with its citation. |
| S5 | ✅ for a hub origin or `--from` the payload is fetched at `<origin>/blob/<sha256 of the pin>` and `payload.url` is ignored; internet bases use `payload.url`; sha256 and size checked either way; `record_success` only by an internet-base check; hub and `--from` answers go to the per-origin record `/api/v1/update/origins` shows. |
| S6 | ✅ the carrier builds its own `ureq::Agent` as `wan_probe` does; 5A requests in `spawn_blocking` with connect and global timeouts; the 5B stream in one named std thread (`111d-ntfy-5b`) with no global timeout and a read timeout of 3× the server keepalive (from the smoke log), reconnecting with back-off; §16 gains a threads row (5B: 1). |
| I2 | ✅ every Layer 7 consent (ntfy and chatmail enable, the plain-pair warning, the public gateway bit, origin serving, a metered cap) answers `409 consent_required` with `consent` in the pinned `ConsentRequired` shape; `action` gains `carrier_enable`, `pair_plain`, `gateway_bit`, `origin_serve` (appended, in the pin list); the repeat carries `accept_warning`; `pair_warning_required` dropped. |
| I3 | ✅ `needs_hub` (not `not_hub`); `no_such_transport`, `licensed_board`, `metered_cap`, `lan_off` appended to the enum; the pin list written as the conventions do (six items with file:line, `Proposal.kind` 8 → 9, the SSE events and schemas, `gen-sse.mjs`, the store switch, the contract tests, the Rust mirrors), ending "the change is not done until all are green". |
| I4 | ✅* in v1 every write to `presence.heartbeat.<transport>` answers `409 no_such_transport` except `rns`, which is refused with a pointer to `PATCH /bridge/settings` (Layer 6 owns the LXMF interval; no write-through); the `presence.interval` proposal is refused `unsupported_action` in v1; §19 records "the switch is visible and inert until a sealed LoRa link (v1.1)" as a deviation for the owner; G8 asserts the exact codes. |
| I5 | ✅ the §19 entry: members-only gateway advertisement in v1 is the offer at enrolment and the forwarding reason; existing members and contacts learn nothing until the v1.1 sealed query; said in §7.5 and §3.8 (the badge is the hub's own, never a member's view of its hub). |
| I6 | ✅ a human-only `POST /api/v1/nearby/hops` → 202 (`409 component_unavailable` while the Reticulum component is off; `429 rate_limited` within 30 s of the last run); one `rnpath -t -j` through the rnstatus runner and its timeout; output parsed as untrusted JSON into `hops_away` of the reticulum source only; published on the nearby frame; never on a GET; G8 asserts one spawn per POST and none on GET; the route in §13.1 and the Layer 6 list in §18. |

## Minors

All forty accepted as proposed (C10–C18, S7–S27, I7–I16), with these variations:

| id | variation |
|---|---|
| C15 | ✅* the limit stated in §5.2 and shown in the fabric view ("too large for the carrier: N B > 2,937 B"); the honest wire status is a §17 item. |
| S10 | ✅ the feature claim marked unverified from this box; G11 asserts `cargo tree -e features` shows no `aws-lc-rs` and no second provider; the sign-off row named in `1-bootstrap/TASKS.md` beside the `LICENSING.md` §5 note. |
| S20 | ✅ the ntfy carrier is a switch in the §2.1 rule-2 sense (zero weight off), not a §2.1 component: no `assets/components/` entry, no payload, consent in `state/relay.json`; the chatmail carrier (§6.5) rides the Delta Chat component. |
| S27 + I16 | ✅ the §19 entries added and numbered consistently from D12 on (the editor numbers them and cross-references): the carrier in-process in v1 with the loopback JSON-lines pipe as the v1.1 external-carrier form (brief §3 resolved); C8 → role bits only in v1; C9 → the link key pinned from the `111PAIR:` bundle, not the card (S21); the I4 and I5 entries. |
| I9 | ✅* bits 3–5 and the name map defined in node111 (`N/fabric/sealer.rs` beside appdata; `roles(s)` at `N/fabric/mod.rs:334-343`): no mesh111 change. |
| I12 | ✅* the 9-min floor as a node111 unit test of the validator with a phone category; the licensed assertion through the Layer 6 bridge stub in G8 and G10 item 6. |
| I15 | ✅* the pairing writes are loopback-only, like their Layer 5 siblings; the CLI `111 fabric pair` covers the phone case. |

## The drafter's deviations (D1–D11) and chosen numbers

| id | disposition |
|---|---|
| D1 | ❓ Q1 → accepted as a code seam (a type generalisation of `serve`'s stream parameter in `M/link/tcp_lan.rs`, byte-identical on the wire), recorded in §4.4 and §19; the flip (tunnel-only internet pairing in v1, TLS-RPK deferred) is written beside it. |
| D2, D3, D4, D7, D9 | ✅ as recorded. |
| D5 | ✅* with C1 (b): 128 MiB total, a per-origin share of 16 MiB. |
| D6 | ❓ Q4 → v1.1 as drafted (the two control kinds pinned, not built). |
| D8 | ✅ with S15's wording (a second `netlink::debounced` instance, 2 s, feeding only the carrier check and the mdns re-arm; the spine's instance untouched). |
| D10 | ❓ Q5 → accepted; with S22's rule (the second `ENROLL` rides the connection the first hub opened: `ENROLL` accepted on an attributed inbound connection). |
| D11 | ✅ with S13's consent copy (the outer header's short tag of the two hubs' ids). |
| numbers | ✅ the Termux fallback poll (15 min), the ntfy `cache-duration` (168 h), the epoch origin day (2026-01-01), the no-timer gate's pass line (1.10 × the first recorded baseline) and the browse window (5 s) stay as the lead's defaults, each marked "(a default; the owner may change)". |

## Owner questions (each applied in v0.2 as the lead recommends; a different answer lands in v0.3)

1. **Q1 (D1):** TLS raw-public-key wrapping needs one code seam in mesh111's tcp-lan adapter beyond the twelve Layer 5 amendments (the
   `serve` stream parameter generalised; no wire byte changes). Lead: accept it as a recorded code seam. Flip: internet pairing is
   tunnel-only in v1 and TLS-RPK moves to v1.1.
2. **Q2 (I1, I9, C9):** read-only or byte-identical code seams in mesh111 beyond the twelve (a `Fabric::attributed_on(peer)` accessor so
   Nearby can say "on the LAN" versus "through the hub"; the `PairedHub` record shape inside amendment 10). Lead: accept the principle
   that the twelve are the Layer 5 *spec* amendments and byte-identical code seams are listed in §19 and in the lock entry. Flip: one
   `111` fact per contact with the transport unknown.
3. **Q3 (C4):** in v1 a hub's own messages reach another island only through its own home hub (an always-on hub with no home hub sees
   `queued` for off-island contacts). Lead: accept; the own-send route into the hub's own forward table is v1.1. Flip: a thirteenth
   Layer 5 change in v1.
4. **Q4 (D6):** the sealed registry query (two new Layer 5 control kinds) is pinned now and built in v1.1; until then a member learns a
   hub's gateway willingness only from the offer at enrolment and from the `forwarding` reason (I5). Lead: v1.1 as drafted. Flip: v1.
5. **Q5 (D10, S22):** two hubs must meet once over tcp-lan (the same LAN, a tunnel or one public address) to pair; afterwards they may
   talk only through a carrier. Lead: accept. Flip: pairing over a carrier, which needs a new payload form.
6. **Q6 (C1):** a forward row every reachable target refused is marked `refused` and becomes evictable, and the Transit pool has a
   per-origin share so one origin cannot starve the star. Lead: within F4, accept. Flip: none sensible.
