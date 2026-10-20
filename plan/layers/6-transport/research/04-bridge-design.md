# Layer 6, lens 4: bridge design (categories, fan-out, relay, loop guard, consent, prior art)

Research date 2026-10-19. Web facts carry a URL; code facts carry file:line (tree at `6ae61a0`). `(unverified)` = not read from a primary source. Lens 0 (`00-code-seams.md`) is the seam map this builds on; its store pick (A) and routes are reused, not repeated.

## 1. Summary

- **Build the bridge core in 111d (Rust); adapters are thin and only move native text.** No prior-art project fits as the core. The closest, matterbridge, has no Meshtastic or LXMF backend, keeps no per-target delivery state, clips text by characters, and holds its ID map in memory. Its patterns (star gateway, nick-prefix template, API gateway) are worth copying, not its code.
- **Foreign messages get their own tables, never NodeId-shaped rows.** `messages.peer/sender` are `CHECK(length = 32)` (`50-messaging/crates/mesh111/src/store/schema.rs:71-72`). Add V3 tables (conversation, message, relay edge, send group) in the same SQLCipher store. This needs the owner's OK to amend locked L5.
- **Loops are unsolved in the prior art, so the guard is ours.** Every Meshtastic relay found tags text with a prefix, and only one even says "already relayed does not recirculate". The plan: relay edges are explicit, directed and non-transitive; a visible `[name/TAG] ` marker (a structured LXMF field on that side); an ID map; own-echo suppression. A relayed copy is never relayed again.
- **Fan-out is per-target messages in one send group**, each with a preview (bytes, parts, airtime fact, refusals), its own state and retry. Default refuse over-size; split is an explicit per-send choice; silent truncation never.
- **"Delivered" is honest only with recipient evidence.** A Meshtastic channel broadcast or an LXMF propagated send can reach `relayed` at most. Each state carries an `evidence` string the UI prints.
- **Consent is structural.** Only channels and groups can be relay endpoints (DMs are unrepresentable); only a human on a paired or loopback session enables an edge; the agent proposes.

## 2. Candidates

| Name | Upstream | License (verified?) | Lang | Maturity | Fit | Why |
|---|---|---|---|---|---|---|
| matterbridge (original) | github.com/42wim/matterbridge | Apache-2.0 (yes: GitHub metadata + README) | Go | v1.26.0; last push 2024-12-12; 7.6k stars | reference | Star gateway, `RemoteNickFormat`, REST API gateway for custom clients. No Meshtastic/LXMF protocol. Stale. |
| matterbridge-org fork | github.com/matterbridge-org/matterbridge | AGPL-3.0 after commit 20988f6, Apache-2.0 before (yes: README + metadata) | Go (needs Go 1.26) | active; last push 2026-10-09; volunteer-run | reference | Same design. Possible later Layer 7 internet-chat gateway as a separate process. Not the core. |
| mmrelay (Meshtastic-Matrix relay) | github.com/jeremiah-k/meshtastic-matrix-relay | GPL-3.0-or-later (PyPI), GPL-3.0 (repo page); file not read | Python 3.11+ | 1.4.1 on 2026-10-14; releases roughly monthly | reference | Prefix `{display5}[M]: `, `message_delay` floor 2.0 s, channel to room 1:1. Loops and DMs not documented. |
| MTMC Bridge | sourceforge.net/projects/mtmc-bridge | GPLv3 (project page only) (unverified) | Rust | beta; updated 2026-08-08 | reference | Meshtastic and MeshCore public-channel star relay, `[name] message`, "already relayed does not recirculate". |
| Meshenger | github.com/meshbehave/meshenger | MIT per README; GitHub license field is null (unverified) | Rust | 46 commits; pushed 2026-02-21 | reference | Rust TCP client on a node; `[TG:alice]` prefix; per-module public/DM/both scope; queue with pacing and jitter. |
| LXMF-Tools `lxmf_bridge_meshtastic` | github.com/SebastianObi/LXMF-Tools | MIT (yes: GitHub metadata) | Python | beta; last push 2025-03-19 | reference | The only Meshtastic to LXMF text bridge from a known author; README is a stub. Almost certainly imports RNS/LXMF (source not read) (unverified); rule 1 makes it pattern-only. |
| Splice | codeberg.org/drkhsh/splice | unknown (unverified; page unreachable) | Python (lxmfy, paho-mqtt) | commits May 2026 (search summary only) | reference | Meshtastic channel to LXMF group; MQTT in, gateway HTTP out; member list in config; holds no channel key. |
| matrix-lxmf | git.rns.moscow/deuxyeux/matrix-lxmf | none chosen (README) | Python | 14 commits; 2026-10-13 | reject | No license. Pattern: portal room per LXMF contact plus ghost user. Needs a shared rnsd instance. |
| mautrix bridges | github.com/mautrix/whatsapp | AGPL-3.0 (yes: LICENSE file) | Go | active | reference | Relay mode (per-room opt-in, name prefix) and portal/ghost vocabulary. Matrix-hub design, not adoptable. |
| biboumi | codeberg.org/poezio/biboumi | Zlib (page statement) | C++20 | commit 2026-10-11 | reference | XMPP to IRC gateway whose README says it maps partially on purpose and enforces the target's rules. |

## 3. Findings that change the plan

1. **No chat-bridge core has a Meshtastic or LXMF backend.** The 42wim README lists 19 protocols without either (https://github.com/42wim/matterbridge); the fork's `docs/protocols/` has none (https://github.com/matterbridge-org/matterbridge/tree/master/docs/protocols). Its REST API (`POST /api/message`, `GET /api/stream`, bearer token) is the only extension point (https://raw.githubusercontent.com/wiki/42wim/matterbridge/API.md).
2. **Matterbridge's loop safety is topological, not tagged.** `SendMessage` skips the channel the message came from (`channel.ID == getChannelID(rmsg)`); routes are scoped to one gateway; there is no own-user check, and the Matrix bridge warns to use a separate bot account (https://raw.githubusercontent.com/42wim/matterbridge/master/gateway/gateway.go, https://github.com/42wim/matterbridge/wiki/Settings). The ID cache is an in-memory LRU of 5000 entries for edits and replies, and Slack threading notes it is flushed on restart. A star works while one bridge owns the gateway; it breaks when two bridges join the same channels.
3. **Matterbridge clips by default.** `MessageLength` 400 characters, then `<message clipped>`; `MessageSplit` is off and the docs warn it "could lead to flooding"; `MessageQueue` 30 drops overflow (https://github.com/42wim/matterbridge/wiki/Settings). Characters are not bytes, and drops are silent: the opposite of our delivery-first rule.
4. **Meshtastic relays attribute with an in-text prefix, spending bytes.** mmrelay default `{display5}[M]: `, "10 characters leaving ~200" (https://raw.githubusercontent.com/jeremiah-k/meshtastic-matrix-relay/main/docs/ADVANCED_CONFIGURATION.md); MTMC `[name] message` (https://sourceforge.net/p/mtmc-bridge/); Meshenger `[TG:alice]` (https://github.com/meshbehave/meshenger). None documents a loop guard beyond MTMC's one sentence, and mmrelay's DM handling is undocumented (unverified).
5. **Size facts.** `Data.payload max_size:233` (https://raw.githubusercontent.com/meshtastic/protobufs/master/meshtastic/mesh.options); the Android composer caps at 200 bytes, a UI choice (https://meshtastic.org/docs/software/android/user/messages-and-channels/); about 221 B with PKI (`00-knowledge/projects/meshtastic.md:64`). LXMF opportunistic content is at most 287 B, a link packet 319 B, larger as a resource (https://raw.githubusercontent.com/markqvist/LXMF/master/LXMF/LXMessage.py).
6. **Meshtastic gives flow-control and refusal facts.** `QueueStatus{res, free, maxlen, mesh_packet_id}` reports acceptance and free queue slots; `Routing.Error` includes `TOO_LARGE`=7, `DUTY_CYCLE_LIMIT`=9 and `RATE_LIMIT_EXCEEDED`=38 ("airtime fairness ... per portnum"; which portnums it hits is unverified) (https://raw.githubusercontent.com/meshtastic/protobufs/master/meshtastic/mesh.proto). With no region set the node transmits nothing; a node at its duty-cap stops transmitting (EU_868 10%, EU_866 2.5%) unless overridden (https://meshtastic.org/docs/configuration/radio/lora/).
7. **Meshtastic delivery evidence is thin by design.** Broadcasts get no ACK: a heard rebroadcast is the implicit ACK, up to 3 sends; a DM gets a real ACK from the recipient, up to 5 sends, then an internal NAK (https://meshtastic.org/docs/overview/mesh-algo/). The Android app prints "Delivered to mesh" for broadcasts and "Delivered to recipient" only for DM ACKs (messages-and-channels page above).
8. **LXMF states and fields.** `OUTBOUND, SENDING, SENT, DELIVERED, REJECTED`; a propagated send stops at `SENT`; `DELIVERED` needs the recipient's confirmation (LXMessage.py above). Fields `FIELD_REPLY_TO` 0x30, `FIELD_REACTION` 0x40, `FIELD_GROUP` 0x0B and `FIELD_CUSTOM_TYPE/DATA/META` 0xFB-0xFD exist (https://raw.githubusercontent.com/markqvist/LXMF/master/LXMF/LXMF.py). So the relay marker costs no visible bytes on the LXMF side.
9. **Meshtastic channel identity is name plus key, not index.** `MeshPacket.channel` is "only meaningful locally"; matching name and PSK define a channel (mesh.proto above; https://meshtastic.org/docs/configuration/radio/channels/). Channel-message senders are unauthenticated: PSK channels are "encrypted but not authenticated" (`00-knowledge/projects/meshtastic-pki.md:25-28`). `Data` has native `reply_id`/`emoji` and a `bitfield` whose first use is "user approves the packet being uploaded to MQTT"; the bit position is unverified.
10. **Meshtastic's own consent convention exists.** `ok_to_mqtt` is "a polite request that is enforced in the official firmware", for default-key channels only; `Ignore MQTT` drops packets that passed through MQTT (lora docs above). Community norms (regional, not official): bots reply by DM, avoid the default public channel, say `bot` in the node name, do not answer other bots (https://areyoumeshingwith.us/docs/general/bots-and-automation/).
11. **Relay-mode precedent.** mautrix relay: per-room opt-in command, admin-only by default, sender name prefix, reactions from relayed users are not bridged (https://docs.mau.fi/bridges/general/relay-mode.html). Matrix tells bots to ignore `m.notice` messages to stop bot loops (spec mirror, wording unverified: https://git.sudo.is/matrix/matrix-doc/src/branch/dbkr/add_dot/event-schemas/schema/m.room.message%23m.notice). biboumi: "doesn't try to provide a complete mapping" and tells the user the target's rules (https://codeberg.org/poezio/biboumi).
12. **The Meshtastic to LXMF text bridges that exist** (LXMF-Tools, Splice) are Python programs on the RNS/LXMF stack (Splice uses `lxmfy` per its search summary; LXMF-Tools source not read) (unverified). Rule 1 makes them pattern-only; Splice's useful idea is that the radio keeps the channel key and the bridge never holds it.
13. **Hooks in our tree.** L5 §14 and `N/meshtastic/mod.rs:6-7` still say "111 never transmits"; D23 supersedes both. A10 lets a 111 node recognise another 111 bridge on a foreign network, but `TransportId` has only nostr `0x01` (`20-identity/crates/id111/src/attest.rs:150-170`, via lens 0 finding 11). `Proposal.kind` is a closed enum (`40-interface/api/openapi.yaml:556`).

## 4. Recommendation

### 4.1 Foreign message model and where it lives

Model (all text untrusted, rendered as text only):
- **Conversation:** `network` (`meshtastic`, `lxmf`, `app:<id>`), `account` (which board or identity of ours), `kind` (channel | dm | group), `native_id`, title.
  - Meshtastic channel: `ch:<name>:<psk-fingerprint>`; DM: `dm:!<nodenum>`.
  - LXMF: `dm:<dest-hash>`.
- **Message:** `dir` (in | own), `sender_native` (opaque), `sender_name` (untrusted, ≤64 B, control characters stripped), `auth` (`none` | `channel-key` | `pki` | `signed` | `attested-111`), `native_msg_id`, `body`, `local_seq` (receipt order, never claimed time), `claim_ts` (foreign hint, display only), `state`, `evidence`, `reason_code`, `group_id`, `origin_msg` + `edge_id` (set on relayed copies), optional `meta` (snr, rssi, hops).
- **Dedupe key:** `UNIQUE(conv, sender_native, native_msg_id)`.

| Option | Trade-off |
|---|---|
| A. V3 tables in the mesh111 SQLCipher store, typed `Fabric::foreign_*` calls, `Class::Foreign` in the §10.4 prune order | One key, one prune pass, one `local_seq`; amends locked L5 |
| B. Separate bridge store in node111 | L5 untouched; second key, second prune, second ordering |
| C. Memory-only | Loses history on restart; contradicts "people will live in the dashboard" |
| D. Pseudo-NodeIds in `messages` | Rejected: enters ACK/contact logic and breaks honest labels |

**Pick A.** Prune rule (capacity only): after superseded objects, before own history, foreign inbound not pinned, size times age; never foreign outbound undelivered or a conversation with an enabled edge's unrelayed copies.

### 4.2 Categories in Messages

| Option | Trade-off |
|---|---|
| a. A network rail or tabs: "111" first, then one per network, each split into channels and DMs; thread view per conversation | Matches D23 "keep it categorized"; new route `/messages/net/:network[/:conv]` (lens 0) |
| b. One merged inbox with network badges | Violates "never merged into 111 threads" |
| c. Hosted panels only | Does not meet the owner's goal |

**Pick a**, plus an optional read-only "All networks, newest first" feed later (rows carry the network chip; never a thread).
- **Each foreign message shows:** the network chip, `!a1b2c3d4` or the hash, the untrusted name as text, and an `auth` label ("not authenticated: anyone with this channel key can use any name").
- **Each network banner shows adapter-supplied honest copy** (Meshtastic: the default key is public; nothing is 111-sealed).
- **"Message them":** a Meshtastic node or LXMF address is never a contact. The action reads "Send a Meshtastic DM from this node (radio, readable, not 111-sealed)" and opens a foreign DM conversation. "Open the sealed thread" appears only when an A10 attestation or an in-band 111 card links that address to a NodeId. Nearby gets the same button.
- **Labels come from an adapter manifest** (name, copy, chip token, capabilities), so no per-network hard-coding (D17 agentic principle).

### 4.3 Compose fan-out

| Option | Trade-off |
|---|---|
| a. Targets picker; one `send_group`; one message row per target; preview per target; per-target size mode | Honest, per-target retry/cancel; more UI |
| b. Cap every target at the smallest limit (about 200 B) | Simple; wastes LXMF and 111 capacity |
| c. Auto-truncate or auto-split | Silent change of the user's words; Meshtastic parts arrive unordered |

**Pick a.**
- **Preview** (a no-side-effect call): UTF-8 bytes, parts, airtime estimate from the adapter's advertised profile and the KB time-on-air table (255 B on LongFast is about 2.1 s), region/TX status, and refusals ("312 B > 233 B per packet").
- **Size modes per target:** `refuse` (default) or `split` (opt-in per send, parts numbered `(1/3) `, overhead counted in the preview). Truncation only as an explicit user edit.
- **Isolation:** a failure on one target never cancels the others. Pacing is the adapter's, driven by `QueueStatus.free`, not a policy cap.

### 4.4 Relay between networks and the loop guard

| Option | Trade-off |
|---|---|
| a. Explicit directed edges (conversation to conversation), non-transitive, enabled by a human | Loop-free by construction; A to B plus B to C does not give A to C |
| b. Gateway set like matterbridge (all members see all) | Easy mental model; breaks with two bridges on one channel |
| c. Relay everything automatically | Rejected: consent and airtime |

**Pick a.** A bidirectional pair is two edges; the UI may offer "link these two" as sugar. Guard layers, all required:
1. **Never re-relay a relayed copy.** Our own sends are recorded (Meshtastic id learned from `QueueStatus.mesh_packet_id`; LXMF message hash) and flagged `origin_msg`.
2. **Own-echo suppression.** Inbound with `from` equal to our bridge node, or source hash equal to our LXMF destination, is dropped as inbound. Whether the firmware ever hands us our own rebroadcast is unverified; the guard assumes it can.
3. **Visible marker** `[<name≤10>/<TAG>] ` (TAG one to three ASCII letters, registered per adapter: `M` Meshtastic, `LX` LXMF). Any 111 node treats a text starting with it as already relayed. Cost is 8-14 B on Meshtastic. On LXMF the same data rides `FIELD_CUSTOM_TYPE="111.relay"` plus `FIELD_CUSTOM_DATA`, with no prefix text in the body.
4. **Foreign bridges** (mmrelay, matterbridge shapes) are recognised only as optional "looks relayed" hints, default off. Chasing every prefix is fragile.
5. **Two 111 relays on one pair** would duplicate but not loop (the marker stops the loop). v1 detects an attested 111 bridge posting on a pair and warns "already bridged by node X" when the user enables the edge. Coordination between the nodes over the sealed fabric (lowest NodeId relays) is a later option.
6. **ID map bounds:** capacity and `local_seq` distance, never wall-clock. A restart keeps it, because it lives in the store.

Fan-out of a 111 message into a foreign network happens only by the user's explicit act (4.3); an edge never carries 111 sealed threads.

### 4.5 Consent and privacy

- **DMs unrepresentable.** An edge endpoint must be `kind in {channel, group}`; the type forbids a DM. Private DMs are never auto-relayed.
- **Who enables.** Only a paired or loopback human session ("loopback or a paired session, like every write", `40-interface/api/openapi.yaml:47`). The agent proposes `bridge_edge`; accepting records the exact warning text shown.
- **Warning text at enable time.** "Everyone in channel X will have their messages copied to Y. They see a prefix on Y only if they read Y." The default key is public. Optional courtesy notice on the source channel ("this channel is bridged by node N"): costs airtime, owner decides (Q2).
- **Opt-out bits.** Meshtastic `ok_to_mqtt`: honoured as a sender's refusal to leave the mesh (default on, public default-key channels only) (Q3).
- **Region.** Readable relay into an amateur-licensed radio needs a human confirmation: third-party-traffic rules are the licensee's fact, never guessed by the agent. Sealed frames stay off HAM mode (devices.md §0.6).

### 4.6 Airtime honesty

111 sets no rate or expiry limit. It shows facts: per-send airtime, region duty-cap, an edge's backlog (`QueueStatus.free == 0` reads "waiting for the radio"), and firmware refusals printed with their reason (`duty_cycle`, `too_large`, `region_unset`, `rate_limited`). An edge's live counters let the agent suggest turning it off; it cannot do so. A relay backlog is held in the store up to capacity; shedding oldest copies is capacity-only, counted and shown on the edge, never silent (Q6).

### 4.7 Where the core lives

| Option | Trade-off |
|---|---|
| A. 111d bridge core; Meshtastic adapter in-binary on the meshmud downlink (D22 allows GPL-3.0); LXMF and phone-app adapters as hash-locked sidecars over a spool dir (lens 0) | One consent/loop/state implementation; stub adapters testable in netns; no extra runtime on Termux |
| B. Logic inside each sidecar | Loop guard and fan-out cross sidecars anyway; duplicated policy |
| C. matterbridge-org as the core with API-gateway clients | Gets prefix templating and edit/reply mapping; but persistence, consent, byte limits and per-target state are ours anyway; a Go AGPL daemon needing Go 1.26 sits between adapter and 111d as overhead |
| D. A, plus matterbridge later as an optional Layer 7 internet-chat gateway (a separate connected service) | Keeps the door open without the cost now |

**Pick A (D later).** The adapter contract is small: `caps()` (conv kinds, `max_text_bytes` per kind, evidence kinds, airtime profile, auth levels), `recv()` (inbound foreign message), `send(conv, text)` returning events `Accepted(native_id)`, `Evidence(kind)`, `Failed(reason)`. A stub adapter implements it in-process for the netns tests.

### 4.8 Agent tools and the additive contract

- **Tools:** `bridge.read` (networks, conversations, recent foreign messages inside a labelled untrusted block, edges, adapter facts); `messages.send` stays a proposal and gains optional targets; `bridge.propose_edge`. `Proposal.kind` gains `bridge_message` (body: `to:` lines naming network and conversation, a blank line, the text verbatim) and `bridge_edge`. Both need the closed-enum bump (lens 0 f).
- **Routes (extends lens 0):** `GET /networks`; `GET /networks/{net}/conversations[/{conv}]`; `POST /networks/{net}/conversations/{conv}/messages`; `POST /messages/fanout/preview` and `POST /messages/fanout` returning `{group_id, items:[{target, id, plan}]}`; `GET|POST|DELETE /bridge/edges`.
- **SSE:** `network_message` (coalesced per message per tick like `message`) and `bridge` (adapter and edge status). Unknown events are ignored by the UI today (lens 0 a), so the change is additive.

### 4.9 Delivery states for foreign targets

States stay `queued | sent | relayed | delivered | failed`, plus an `evidence` string the UI prints:

| Network and mode | queued | sent | relayed | delivered |
|---|---|---|---|---|
| Meshtastic channel | board away or queue full | accepted by the radio (`QueueStatus`) | a neighbour's rebroadcast heard ("on the mesh") | never (no recipients known) |
| Meshtastic DM | same | accepted | next hop confirmed, no ACK yet | the recipient's ACK |
| LXMF direct or opportunistic | `OUTBOUND` | `SENT` | none | `DELIVERED` (recipient confirmation) |
| LXMF propagated | `OUTBOUND` | `SENDING` | `SENT` (a propagation node holds it) | not available |
| Phone app | adapter-declared; reportedly Briar shows one tick (mailbox) and two (recipient) per `00-knowledge/projects/briar-mailbox.md:46` | | | |

Failures map the firmware reason (`NO_ROUTE`, `MAX_RETRANSMIT`, `DUTY_CYCLE_LIMIT`, `TOO_LARGE`) or `REJECTED` to `reason_code`. Relayed copies carry their own state; the origin sender gets no relay receipt.

## 5. Open questions for the owner

1. May V3 foreign tables, `Class::Foreign` and typed `Fabric::foreign_*` calls amend locked L5 (same as lens 0 Q1)?
2. Post a "this channel is bridged by node N" notice on the source channel when an edge is enabled? Costs airtime; default on or off?
3. Honour Meshtastic `ok_to_mqtt` (opt-out of leaving the mesh) as a relay filter by default? It limits bridging on public channels but follows the firmware's own convention.
4. Is a visible `[name/TAG] ` marker acceptable on Meshtastic (8-14 B per relayed message)?
5. Two 111 nodes relaying the same pair: warn only in v1 (my pick), or build sealed relay-claim coordination?
6. Relay backlog: hold to capacity, then shed oldest with a visible counter (my pick), or hold everything?
7. Relay into an amateur-licensed radio: allow after a human confirmation (my pick) or block?

## 6. Sources

Our tree: `os/README.md` §6; `os/50-messaging/README.md` §8, §10.3, §14, §15, §17; `os/50-messaging/crates/mesh111/src/store/schema.rs:43,69-89`; `os/40-interface/api/openapi.yaml` (Thread, Message, Proposal, `sendMessage`); `os/40-interface/ui/src/views/messages/list.tsx`; `os/30-intelligence/crates/agent111/src/tools.rs:121-175`; `os/20-identity/README.md` §10, A10; `os/plan/cross-layer/devices.md` §0, §7-§9; `os/plan/DECISIONS.md` D23; `os/plan/layers/60-transport/research/00-code-seams.md`; KB `lora-airtime-budget`, `meshtastic`, `meshtastic-pki`, `lxmf`, `briar-mailbox`.

Web (all fetched 2026-10-19 unless noted):
- https://github.com/42wim/matterbridge · https://api.github.com/repos/42wim/matterbridge · https://github.com/matterbridge-org/matterbridge · https://api.github.com/repos/matterbridge-org/matterbridge · https://github.com/matterbridge-org/matterbridge/blob/master/README.md · https://github.com/matterbridge-org/matterbridge/tree/master/docs/protocols
- https://github.com/42wim/matterbridge/wiki/Settings · https://raw.githubusercontent.com/wiki/42wim/matterbridge/API.md · https://raw.githubusercontent.com/42wim/matterbridge/master/gateway/gateway.go
- https://github.com/jeremiah-k/meshtastic-matrix-relay · https://pypi.org/project/mmrelay/ · https://raw.githubusercontent.com/jeremiah-k/meshtastic-matrix-relay/main/src/mmrelay/tools/sample_config.yaml · https://raw.githubusercontent.com/jeremiah-k/meshtastic-matrix-relay/main/docs/ADVANCED_CONFIGURATION.md
- https://sourceforge.net/p/mtmc-bridge/ · https://github.com/meshbehave/meshenger · https://api.github.com/repos/meshbehave/meshenger
- https://github.com/SebastianObi/LXMF-Tools · https://api.github.com/repos/SebastianObi/LXMF-Tools · https://zff.dev/MattJ/siltamesh · https://git.rns.moscow/deuxyeux/matrix-lxmf · Splice via search summary only (https://codeberg.org/drkhsh/splice, page returned 404)
- https://docs.mau.fi/bridges/general/relay-mode.html · https://raw.githubusercontent.com/mautrix/whatsapp/main/LICENSE · https://codeberg.org/poezio/biboumi · https://git.sudo.is/matrix/matrix-doc/src/branch/dbkr/add_dot/event-schemas/schema/m.room.message%23m.notice
- https://raw.githubusercontent.com/meshtastic/protobufs/master/meshtastic/mesh.proto · https://raw.githubusercontent.com/meshtastic/protobufs/master/meshtastic/mesh.options · https://meshtastic.org/docs/overview/mesh-algo/ · https://meshtastic.org/docs/configuration/radio/lora/ · https://meshtastic.org/docs/configuration/radio/channels/ · https://meshtastic.org/docs/configuration/radio/user/ · https://meshtastic.org/docs/software/integrations/mqtt/ · https://meshtastic.org/docs/software/android/user/messages-and-channels/ · https://areyoumeshingwith.us/docs/general/bots-and-automation/
- https://raw.githubusercontent.com/markqvist/LXMF/master/LXMF/LXMessage.py · https://raw.githubusercontent.com/markqvist/LXMF/master/LXMF/LXMF.py
