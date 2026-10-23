# Layer 6 — Transport & Bridge

The layer that joins the node to the networks it does not own. A **bridge core** inside `111d` gives every foreign network its own category in
Messages, sends one text into several networks as per-target messages, and relays between networks over explicit, human-enabled, loop-guarded edges.
**Adapters** move native text and nothing else. A **transport spine** (`state/interfaces.toml`, the rnsd renderer, live attach, a phone relay) and a
**device core** (hotplug, port ownership, consented identify, RNode stanza proposals, a narrow apply→verify→undo cut) carry the radios. Every adopted
program is a separate, on-demand component that weighs nothing at runtime while it is off.

Status: ✅ **v1.0, locked (D27, 2026-10-19)** — v0.3 as reviewed (three-judge fable panel on v0.1, one re-check on v0.2; D25, D26). **Built and locked as built on 2026-10-23 (D29):** the build board `TASKS.md` carries every task's as-built facts, the gates, the measured Pi floor, the recorded amendments and the open items; the spec text below is unchanged.
`os/plan/EXECUTION.md` (not written yet). Anything beyond the options brief and research lens 4 §4.8 is marked **(draft proposal)**. The changelog
from v0.1 is at the end.

## 0. Reading guide

Reads with: `os/README.md` §6 · `os/5-messaging/README.md` §8, §10, §13–§15, §18–§21 (the locked v1.2 fabric this layer amends additively) ·
`os/2-identity/README.md` §10, §13.1 A10 · `os/plan/cross-layer/devices.md` (DEV) §3, §7, §9 · `os/plan/DECISIONS.md` D17, D19 c, D20, D22, D23, D25
and **D26** (the law for this draft) · the options brief (OB) and research R00 seam map, R01 Meshtastic, R02 LXMF, R03 phone apps, R04 bridge design,
R05 transports and devices, R06 critic, R07 gap-1 run, all under `os/plan/layers/6-transport/` · its `review/` folder.

**Citations.** `M/` = `5-messaging/crates/mesh111/src` · `N/` = `1-bootstrap/crates/node111/src/daemon` · `A/` = `3-intelligence/crates/agent111` ·
`UI/` = `4-interface/ui/src` · `API` = `4-interface/api/openapi.yaml` · `ID/` = `2-identity/crates/id111/src` · `L5` = `5-messaging/README.md`.
Every `file:line` was re-read in the working tree on 2026-10-19 (v0.3 re-pinned `supervise.rs`, the mesh111 `fabric.rs` lines, the `openapi.yaml`
NearbySource enum and the presence reader after later commits). **Evidence:** *(run)* executed here · *(src)* source read · *(doc)* docs only ·
*(design)* not yet run.

**In one paragraph.** A bridge core in `111d` owns consent, loop guard, fan-out and the foreign tables (mesh111 store, schema V3). Meshtastic goes
through the official crate, LXMF through our own small helper, phone apps through hub-side peers, all as native-text adapters; a transport spine and a
device core carry the radios; third-party programs are downloaded by the user, never bundled. Everything is verified in network namespaces with stubs
(§16).

## 1. Scope & non-goals

**First signed release (R-B, D25 F5), in wave order:**
- **Wave 0, spine:** `state/interfaces.toml`, renderer v2 and posture v2, live attach, `transport.read`; schema V3 and its contract; the A10 byte
  values and verifier plug (§14); the global bearer write gate (§13).
- **Wave 1, Meshtastic:** bridge core, Meshtastic client, Messages categories, fan-out, relay edges, agent tools; Nearby folds into the client and the
  Python presence reader retires after parity.
- **Wave 2:** LXMF through `lxmf-sidecar`, app peers (Delta Chat only after a passed spike, then SimpleX), phone relay v1.
- **Wave 3, device core:** hotplug, consented identify via adopted tools, RNode stanza proposals, the narrow applier; Settings → Layer 6. **Wave 4:**
  netns verification with stubs, then the fable re-check.

**Second release (D25 F6, D26 #3):** the 111 KISS modem link and its manifest-pinned CI image · the MeshCore data-type PR (dev range `FF00–FFFF` until
the bench) · sealed 111 carriage over foreign radios (the six mesh111 amendments of R00 §3 findings 1–7) · attestation gossip and its labels · hosted
panels (§17). The first release carries **readable text only**.

**Hard rules carried in:** adopt, don't reinvent (named at every adapter) · `111d` imports nothing of Reticulum; the one program of ours that does is
the LXMF helper, a separate process in its own venv (D26 #4 amends D17 #12) · no wall clock in any cryptographic or ordering decision (`local_seq` and
the monotonic tick only) · no runtime downloads (components download at install, §2.1) · third-party programs are downloaded, never bundled, and the
user installs them after a licence notice (D26 #5) · no telemetry · no policy limits of ours (README §5) · delivery is non-negotiable · foreign text
is untrusted everywhere · a 111 message enters a foreign network only by the user's explicit act · the agent proposes and a human decides, with **no
exception**: its bearer is refused on every write (§13, D26 #2) · GPL code never links into the Rust binary **except** the D25 F2 crate, which makes
the shipped `111d` effectively GPL-3.0-only (our files stay "or later"; §15).

**Owner rules (verbatim; binding on every adapter below):**
- D25 F3: "try to keep it as clean as possible, separate and on demand running, meaning if it's not running, it doesn't make our system heavier! you
  decide this one, performance is super important + I want users to have 100% freedom."
- D25 F4: "keep it as thin as possible and separate so we don't make the repo heavier, decide yourself. again, the goal is users get hooked to our UI
  but we offer opening the native of other apps as well in our dashboard separate."
- D25 framing: "the whole idea is to ship thin layer of own, handselect a few that run native, not fully decided which yet, then we do a full review
  and integrate." (Which programs run as natives is the owner's hand-selection; nothing in §2.1 depends on it.)
- D23 #2: "the chances are that people will get used to our network fully, even the dashboard. The UI of other services must be available fully, but
  still text appears in ours, in the Messages view, from different networks. Keep it categorized for other networks."
- D26 #4: "lets make our own helper, attribute their work, I will contact them to find out if we are ok."
- D26 #5: "always download, never bundle. also, in the whole licensing, try to make the user responsible and keep me as the author safe … make little
  notice to read their licence etc of that app."

**Where it runs:** Cat 3 and Cat 4 (README layer map); the phone-relay client stanza (§11) is the one phone-side piece. **No hardware (D23 #4):**
every radio path is built and verified against stubs, scripted daemons and real `rnsd` in network namespaces (§16); hardware gates stay in the runbook
and are never reported as passed. **Non-goals:** MeshCore as a bridge target · flashing · writing Meshtastic node configuration · driving a phone's UI
· Briar offline · any hosted panel in R-B · README §6's runtime "join in sequence, fall back" (Layer 7, §11) · any policy rate or size cap.

## 2. Topology & components

| Component | What | Source | Runs when |
|---|---|---|---|
| Bridge core `N/bridge/` | adapter registry, ingest, fan-out, relay edges, delivery state, component runner, API, SSE | custom glue (Layer 1 crate) | always; inert with no adapter enabled |
| Meshtastic adapter | in-binary client on meshmud's downlink `127.0.0.1:11230` (`N/meshtastic/meshmud.rs:30`) | adopted: `meshtastic` crate 0.1.9 (GPL-3.0) over prost; fallback: our ~300-line prost client (OB F2 A1) | while service `meshtastic` is LIVE |
| LXMF adapter | `lxmf-sidecar`, JSON Lines over piped stdio (§6) | ours: `6-transport/sidecars/lxmf-sidecar/`; imports user-installed `rns`, `lxmf` from PyPI | only when the Reticulum bridge is enabled |
| App-peer adapters | Delta Chat `deltachat-rpc-server` (MPL-2.0) + Madmail (AGPL-3.0); SimpleX (AGPL-3.0) | adopted, separate processes, downloaded at install | per adapter, on enable |
| Transport spine · device core | `interfaces.toml`, renderer v2, applier, `transport.read` · hotplug, ports, identify | custom glue; `rnsd`, `rnstatus`, `rnodeconf` adopted | always (identify only on consent) |

**Ports (loopback unless stated), extending L5 §2:** dashboard 11180 · model 11181 · agent 11182 · fabric 11183 · panels 11190–11229 · meshmud 11230 ·
**new (draft proposal):** 11232 SimpleX CLI WebSocket only. The LXMF helper needs no port. Madmail serves the standard chatmail ports (SMTP 25, 465,
587, IMAP 143, 993, HTTPS 443; *doc*, pinned at the spike), which need `CAP_NET_BIND_SERVICE` (§18) and cannot be had on Termux; `smp-server` uses its
documented default 5223 (*doc*; LAN form `--ip`). rnsd's Backbone listener is 4242 on the LAN address, only while LAN sharing is on. Radio ports are
claimed in the `PortTable` (`N/ports.rs:106`); a loopback listener is claimed there too (precedent: the meshmud downlink, `N/meshtastic/mod.rs:676`)
**and** pre-bound with a listen check before the child starts, because only a bind detects a foreign holder. A held port answers `409 port_busy`.

**State (`/var/lib/111`), additions (draft proposal):** `state/interfaces.toml` (0600, §11) · `state/bridge.json` (0600: recipient lists, per-channel
send switches, HAM confirmations with the text hash, announce name and interval, licence acknowledgements; invalid ⇒ moved aside like
`fabric/settings`) · `secrets/` (0700; 0600 files minted from the OS CSPRNG, never seed-derived, never on any API: `rns-rpc-key`, `ifac-passphrase`;
the tree has no vault, this is the minimal one) · `bridge/lxmf/` (0700: the helper's client `rns/` dir, `identity`, `store/`) · `bridge/deltachat/`
(0700) · `venvs/lxmf` (a link to a complete build in `venvs/lxmf.d/`, the L1 presence-venv pattern) · `components/<id>/` (installed binaries, after
enable) · `cache/packages/<id>/` (install-time payloads) · `fabric/display-times.foreign` (§3). The foreign tables are in `fabric/store.db`.

**Process shape.** The bridge core and the Meshtastic client are in-process tasks. `rnsd` and `meshmud` are the existing supervised children. New
children, all `supervise.rs` `ChildSpec`s (`N/supervise.rs:55`) started only on enable: `lxmf-sidecar` and `deltachat-rpc-server` (piped stdio, §7),
`madmail`, `simplex-chat` + `smp-server`. **Child ordering is bridge-core orchestration, not a supervisor feature** (`supervise.rs` has one
independent back-off loop per child and no dependency gate); the rules are in §6 "Start order".

### 2.1 The optional-component seam (normative)

An **optional component** is anything third-party that is not part of the thin base: the LXMF helper's closure, Madmail, `deltachat-rpc-server`,
SimpleX.
1. **Declared, not present.** The release carries `assets/components/<id>.toml` (hash-pinned, parsed at start with `deny_unknown_fields` like
   `assets/panels/<id>.toml`; an unknown key or invalid file ⇒ not registered): `id`, `adapter`, the payload pin (a hash-locked requirements file for
   Python closures; URL, sha256 and size in the signed manifest for binaries), `[requires]`, `[run]` (ports, args, health), `[weight]` (cached and
   installed size, RSS), `[license]` (SPDX, where to read it). The release carries **no third-party bytes**. **(draft proposal)** for the format.
2. **Zero weight is runtime weight (D26 #1).** A disabled component has no process, listener, loaded code, timer, probe or open file, and nothing is
   installed: no `components/<id>/`, no `venvs/<id>`. Its cached payload may sit in `cache/packages/<id>/` with its size shown ("N MB cached, not
   installed"). `GET /networks` lists it as `disabled` with those facts. G7 asserts it (§16). The one compiled-in exception is the Meshtastic adapter
   (+372 / +332 KiB, no runtime cost unless service `meshtastic` is LIVE).
3. **Install-time cache.** `111 install`, when online, pre-downloads each declared payload into `cache/packages/<id>/` (pip closures with the build
   venv's own pip, binaries from the pinned URL), the rns and presence pattern (`1-bootstrap/crates/node111/src/install/packages.rs`, `pip_closure`),
   after printing one notice line per component (name, licence, where to read it; a Layer 1 UX item, the line the rns venv gets too), and records that
   notice (program, licence, URL) in the install receipt beside the artifact hashes, so the acknowledgement exists before any download. A USB drop
   into the same directory is the offline alternative. Enable installs from the cache offline. **There is no runtime fetch.** A changed pin on a 111
   update reaches the root-owned cache as the updater's normal manifest refresh, the precedent `LICENSING.md` §9.3 sets for the rns pin; no new
   mechanism.
4. **Enabled only by a human** (paired or loopback session; the agent can only suggest). Enable runs: **licence notice** (program, licence, where to
   read it, cached size; the user acknowledges and `state/bridge.json` records it: the user installs, 111 is not the distributor) → check `[requires]`
   → verify cached files against the pin, fail closed → install (`--require-hashes` venv build, or unpack into `components/<id>/`) → start → health.
   Without the recorded acknowledgement nothing is installed or started (`409 consent_required`).
5. **Disable** stops the child and releases its ports and claims; "remove files" deletes the installed tree, `bridge/<adapter>/` state (the Delta Chat
   account store included) and, separately, the cached payload. Nothing is deleted without that click. Weights shown before enable: cached and
   installed size from `[weight]`; the rns closure is existing, the helper's pure wheels plus `cryptography` are *not measured*, and Delta Chat,
   Madmail and SimpleX sizes are recorded at pin time.

## 3. The foreign message model (normative)

Foreign messages have no home in the built tree: `messages.peer`/`sender` are `CHECK(length = 32)` NodeIds (`M/store/schema.rs:71-72`) and the
contract's `Message`, `Thread`, `SseMessage` and the route `/messages/:peer` are NodeId-keyed (R00 §1). A pseudo-NodeId would enter the key cache, the
ACK and contact logic and break honest labels (rejected, D25). Foreign messages get their own tables.

**Schema V3 (D25 F1; additive amendment of L5 §10.3).** `SCHEMA_VERSION` 2 → 3, one appended migration (`M/store/schema.rs:43,46`); a V3 store opened
by a V2 build is refused by the existing check (`migrate`, `StoreError::SchemaTooNew`, `schema.rs:190-197`). Five tables:

| Table | Columns |
|---|---|
| `foreign_conversations` | `id`, `network` (`meshtastic` · `lxmf` · `app:deltachat` · `app:simplex`; with `111` for the fabric these are the network ids, defined here once), `account` (which board or identity of ours), `kind` (`channel` · `dm` · `group`), `native_id`, `title`, `last_seq`, `unread`, `pinned`; `UNIQUE(network, account, kind, native_id)` |
| `foreign_messages` | `id`, `conv`, `dir` (`in` · `own`), `sender_native`, `sender_name`, `auth`, `native_msg_id`, `body`, `local_seq`, `claim_ts` (foreign hint, display only), `state`, `evidence`, `reason_code`, `group_id`, `part_k` + `part_n` (**one row per part** of a split send), `origin_msg` + `edge_id` (set on relayed copies), `meta` (snr, rssi, hops, `via_mqtt`, `ok_to_mqtt`, `truncated`, `handoff`); an index on `(conv, sender_native, native_msg_id)`, not a UNIQUE (dedupe below) |
| `relay_edges` | `id`, `src_conv`, `dst_conv`, `src_kind` + `dst_kind` (denormalised, `CHECK IN ('channel','group')`: a DM endpoint is impossible in the table), `enabled`, `paused`, `cursor_seq` (`local_seq` at enable), `relayed_seq` (highest origin `local_seq` handed to a copy; advanced when a copy is shed too), `honour_ok_to_mqtt` (default 0), `oversize` (`refuse` default · `split`), `consent_hash`, `enabled_by`, counters `relayed`, `shed`, `refused` |
| `foreign_seen` | `edge`, `conv`, `sender_native`, `native_msg_id`, `local_seq`: the loop guard's keys, bounded per edge (4,096, oldest evicted). Bodies are prunable; the guard needs only keys |
| `send_groups` | `id`, `local_seq`, `text`, `targets` (JSON, §9); per-target rows are `foreign_messages` with `group_id` |

**Order and time.** Foreign rows draw `local_seq` from the store-wide counter (`M/store/mod.rs:517`): one key, one prune pass, one order; no clock
orders or decides anything. `claim_ts` (the foreign hint; the LXMF timestamp is the sender's unverified `time.time()`, R02 §3.9) shows only behind a
"sender's claim" disclosure. The "received" time is a display fact in `fabric/display-times.foreign`, the existing record format in a second file so
it cannot collide with 111 row ids (`N/fabric/times.rs:1-13`).

**Typed writes, batched.** `Fabric::foreign_ingest`, `foreign_set_state`, `foreign_queue_send`, `foreign_group_*`, `foreign_edge_*` and reads (draft
proposal names) run in the actor and emit `FabricEvent::Foreign { conv, id, change }` beside `Message`, `Contact`, `Fabric`, `Settings`
(`M/fabric.rs:363`); `with_store` (`:934`) is a trap: it shares `local_seq` and emits no event (R00 §3.2). The fabric is one loop (`run`, `:1021`) and
every call is a synchronous SQLCipher transaction between frame pumps, so `foreign_ingest` takes a bounded batch (256 rows) as one transaction per
adapter tick, the bridge core awaits each call (back-pressure on the actor's input channel), and `Foreign` events coalesce per conversation
(integration M3). G1 carries a 1,000-row burst with a bound on 111 delivery latency.

**Ids.** `fm-<row>` messages, `fc-<row>` conversations, `be-<row>` edges, `sg-<row>` send groups; `MessageId` allows the prefix (`API:686`) and `msg-`
stays 111-only (`N/fabric/json.rs:32`). API bodies and proposals name conversations by `fc-` id, never by native id or name.

**One ingest function (bridge M9).** Every foreign string (names, bodies, the marker name) passes the same function: strip Cc **and** Cf (bidi
controls, zero-width joiners; keep `\n`, `\t`), NFC-normalise, cap (names 64 B at a UTF-8 boundary). Text is stored as that and rendered as text only.
One stored body is capped at 64 KiB; a longer one is stored cut with `meta.truncated` and a visible "message too long to show (N bytes)", never
silently. Attachments are not ingested ("attachment not shown (N bytes)").

**Dedupe (bridge minor 1).** Key `(conv, sender_native, native_msg_id)`. A Meshtastic packet id is 32 bits per board, so a repeat is a duplicate only
when the key **and** the body hash match within the last 32 rows of that conversation (the firmware's own window is 20 ids); otherwise insert. LXMF
hashes are globally unique (R07 §2 check 4) and app-peer ids are the app's: those match on the key alone. Config-stream replays (§5) carry no text and
are ingested idempotently.

**Conversation identity.** Meshtastic channel `ch:<name>:<kh>`, `kh` a keyed BLAKE2b of name and PSK (key derived from the store secret under the tag
`111/bridge-conv/v1`, the L5 §10.2 pattern); the PSK is parsed from `FromRadio.channel`, hashed, dropped, **never persisted, logged or sent to any
API** (R01 §3.5). A re-keyed channel is a new conversation. Meshtastic DM `dm:!<node hex>`; LXMF DM `dm:<dest hash>`. The channel index is board-local
and never part of an id.

**Authentication label `auth`** (closed; copy from the adapter manifest): `none` · `channel-key` ("anyone with this channel key can use any name") ·
`pki` (the board's PKI proved the sender key, not the name) · `signed` (the carrier reports a validated signature: **LXMF only in v1**, the library's
`signature_validated`, R02 §4 C; otherwise LXMF is `none` with the reason) · `attested-111` (reserved for the second release, §14, §17). A foreign
conversation is never a 111 thread and a foreign address never a 111 contact.

**Pruning (amends L5 §10.4).** New `Class::Foreign = 9`, `Class::ALL` grows to 10 (`M/store/rows.rs:32,55`); `Class::from_kind` never produces it
(`rows.rs:68`) and `is_refusable` stays false (`:83`): foreign inbound is evicted, never refused. It has a **shaping quota** (settings key `foreign`:
evict this class first, never refuse), so an MQTT-fed channel cannot pin the store at its watermark (bridge M7). Between steps 2 and 3 of the order
(`M/store/prune.rs:8-22`): **2a** foreign inbound not pinned, size × age by `local_seq` distance · **2b** relay backlog copies (`dir = own`, `edge_id`
set, `queued`) oldest first, each shed counted and `relayed_seq` advanced past it · then step 3. Step 4 "never" gains: a user-composed foreign send
while undelivered, pinned foreign rows, the bounded `foreign_seen` keys. Capacity only.

## 4. Adapter contract (normative)

An adapter moves native text and reports facts. It never decides consent, relaying, loop guard, size policy or persistence, never touches the store,
and its `send` never blocks the bridge core (the fabric awaits `Link::send` inside its actor, `M/fabric.rs:1336`; waiting for airtime there would
stall everything, R00 §3 finding 7).

```rust
pub trait Adapter: Send + Sync {                         // object-safe: futures are boxed
    fn id(&self) -> &'static str;                        // "meshtastic" | "lxmf" | "app:deltachat" | "app:simplex"
    fn caps(&self) -> Caps;                              // static facts + live state; read by preview, UI, agent
    fn recv(&self) -> BoxFuture<'_, Inbound>;            // next native text, conversation upserts, nearby, link state
    fn send(&self, conv: &ConvRef, text: &str, o: SendOpts) -> BoxFuture<'_, SendHandle>; // returns at once
}
pub enum SendEvent { Accepted { native_id: String }, Evidence { state: State, evidence: String }, Failed { reason: Reason, retry: Retry } }
pub struct Caps { kinds: Vec<ConvKind>, max_text_bytes: PerKind, evidence: Vec<State>, airtime: Option<AirtimeProfile>,
                  auth_levels: Vec<Auth>, copy: Honest, recipient_syntax: Option<Syntax>, splittable: bool, state: AdapterState }
```

- **`caps()`**: conversation kinds (Meshtastic channel and DM; LXMF DM plus a local recipient list as a destination-only `group`, since it has no
  channels, R02 §3 finding 16; Delta Chat DM and group); `max_text_bytes` per kind and mode (Meshtastic 232 plain, 220 PKI, 232 on a licensed board,
  §5, the conservative figure assuming the firmware adds the 2-byte `bitfield`; LXMF 287 opportunistic, larger over a link; the Android app's 200 B is
  a convention noted in the copy, not our limit, R01 §3.4); evidence kinds; an airtime profile in L5's `Airtime { duty_cycle_permille, bits_per_s }`
  shape (`M/link/mod.rs:56`) plus the preview's estimate rule; auth levels; the **honest copy** (banner, chip, relay TAG, send warning) from
  `assets/adapters/<id>.toml` (draft proposal), so the UI holds no per-network code.
- **`recv()`** yields texts, conversation upserts, Nearby updates and link state. Inbound queues are bounded; an overflow drops the oldest and a
  counter that `/networks` and the banner show ("N texts dropped while 111 was busy"), never silently (R01 §3.10).
- **`send()`** returns a handle at once; events follow. `Retry` is `Later` or `Never` (the user may retry). **`Later` rows retry on the monotonic tick
  with capped back-off** (bridge M1: a mechanism, not a policy cap, no wall clock) and earlier on an adapter event (link up, peer announce, key
  learned, a free queue slot). The 2.1 s Meshtastic pacing timer is the named timed retry.
- **Monotone states** over the closed enum `ForeignMessageState` = `queued · sent · relayed · delivered · failed` (new; 111's `MessageState`,
  `API:687`, stays at four). `failed` is only for refusals **before air** (`region_unset`, `NO_CHANNEL`, `TOO_LARGE`, `NOT_AUTHORIZED`, `PKI_*`,
  `GOT_NAK`, `adapter_down`) and is terminal unless a later `delivered` arrives. A NAK after the board transmitted (`NO_ROUTE`, `TIMEOUT`,
  `MAX_RETRANSMIT`, `NO_RESPONSE`) leaves `sent` or `relayed` with "transmitted, not confirmed" (bridge M2). `delivered` never regresses.
- **At most once per user act.** Before a handoff the row records `meta.handoff = pending`; `Accepted` clears it. After a crash a `pending` row is
  **never re-sent automatically**: it shows "may have been sent; resend?" (the L5 half-applied pattern). A `pending` row never blocks later rows; a
  resend uses a new packet id (the firmware's 20-id window would make a same-id resend idempotent for recent sends: noted, not relied on).
- **Queue order (bridge M7).** User-composed rows (`group_id` set) go ahead of relay copies, then `local_seq`: ordering, not a cap.
- **`StubAdapter`** implements the trait in-process with scripted accept, evidence, failures, airtime and inbound feed: the fixture for G1 (modelled
  on `M/link/lossy.rs`).

**Delivery states per network** (`evidence` is the string the UI prints; R01 §3.7, R02 §3.10, R04 §4.9):

| Network, mode | `queued` | `sent` | `relayed` | `delivered` |
|---|---|---|---|---|
| Meshtastic channel | "waiting for the radio" (queue full, region unset, board away, retry) | "accepted by the radio" (`QueueStatus.res = 0` for our id) | "a neighbour rebroadcast it" (routing packet with our `from`: an implicit ack) | never: no recipients are known |
| Meshtastic DM | same, plus "key unknown" | accepted | implicit ack ("relayed, not confirmed") | "the recipient confirmed" (a real ack) |
| LXMF direct or opportunistic | `OUTBOUND` ("no path yet", "waiting for their announce") | `SENDING`/`SENT` | none | `DELIVERED` ("the recipient's node confirmed") |
| LXMF propagated (opt-in) | `OUTBOUND` | `SENDING` | `SENT` ("a propagation node holds it") | not available |
| Delta Chat (draft proposal, doc) | pending | handed to the relay | "the relay accepted it" | the recipient's read receipt (may never come) |

**Reasons** (`reason_code`, draft proposal enum extending L5's) are those named in §5 plus `peer_unannounced`, `adapter_down`, `uplink_off`,
`ham_unconfirmed`, `consent_paused`, `cancelled`; `duty_cycle`, `rate_limited` and a DM's post-air NAK are `Retry::Later`.

## 5. Meshtastic adapter (normative)

**Adopt, don't reinvent:** the official `meshtastic` crate (GPL-3.0, 0.1.9; +372 / +332 KiB x86_64 / aarch64 with default features, measured; dropping
its serial and BLE features is *unverified*, and if they stay `serialport` (MPL-2.0) and `btleplug` link in; R01 §3.16) over prost types; `meshmud`
stays the multiplexer and the Meshtastic web client the panel. The crate fits (src, R06 "Settled here"): nonce-0 heartbeat every 5 minutes, a config
id we choose, `connect` over any stream. **One protobuf snapshot:** the crate's own generates the code and the KB region table is built from the same
one; at pin time we confirm whether the crates.io package ships `.proto` files or generated Rust (it changes the §15 row). Fallback: our own ~300-line
client on prost (+152 / +132 KiB, run).

- **One more client.** `N/meshtastic/client.rs` connects to `127.0.0.1:11230` beside the shim's per-session downlinks; no new port claim
  (`N/meshtastic/mod.rs:675-676`). It runs only while service `meshtastic` is LIVE.
- **Handshake once per connection.** `want_config_id` with a random nonzero id never 1, 69420 or 69421; ingest `MyNodeInfo`, `NodeInfo`, `Channel`,
  `Config` (region, preset) until `config_complete_id`. Text arriving before it is buffered (bounded) and mapped once the channel table is complete.
  Another client's `want_config_id` replays the config stream for everyone (meshmud forwards verbatim, R01 §3.10), so replays are ingested
  idempotently and the client never re-sends `want_config_id` inside a session (it can wedge firmware). The nonce-0 heartbeat keeps the 15-minute API
  session alive (R01 §3.9; the presence reader's keep-alive is built, `meshtastic_presence.py:35`). If meshmud's uplink drops, all downlinks close and
  the client reconnects with capped back-off.
- **Send allowlist at the lowest layer** (precedent: the reader's `ALLOWED_TORADIO`, `meshtastic_presence.py:93`). The crate gets a wrapped stream
  that parses every outgoing `ToRadio` and drops, counts and logs (no content) anything outside: `packet` with `portnum = TEXT_MESSAGE_APP` to a
  channel (`to = 0xFFFFFFFF`) or a node · `want_config_id` once · `heartbeat` nonce 0. Never admin, config, `xmodemPacket`, `mqttClientProxyMessage`,
  `disconnect`, heartbeat nonce 1 (the board would broadcast its NodeInfo), any other portnum. The second release adds `PRIVATE_APP`. A unit test
  fixes the list and a grep test fails the build on any other `ToRadio` constructor. G2 includes "a dropped second `want_config_id` does not wedge the
  crate".
- **One send queue.** At least 2.1 s apart on the monotonic clock: the firmware answers text within 2 s of the previous with `RATE_LIMIT_EXCEEDED`,
  per `PhoneAPI` instance, shared with the panel user (R01 §3.8); that pacing timer is the named timed retry. Packet ids are random, nonzero, never
  reused. `QueueStatus.free = 0` pauses the queue ("waiting for the radio"). **`want_ack = true` for channel text and DMs:** the firmware clears it on
  air for broadcasts but keeps the local reliable record that yields the implicit ack only when the client asked (R01 §3.7); G2 asserts the record
  path.
- **Reasons (bridge M2).** Before air, `failed` with the reason: `NO_CHANNEL`, `TOO_LARGE`, `NOT_AUTHORIZED`, `PKI_FAILED`, `GOT_NAK`, and region
  `UNSET` (from `Config.lora.region`; the board transmits nothing). After the board transmitted: `NO_ROUTE`, `TIMEOUT`, `MAX_RETRANSMIT`,
  `NO_RESPONSE` keep `sent`/`relayed` with "transmitted, not confirmed"; a DM's post-air NAK is `Later` on the next NodeInfo from that node.
  `DUTY_CYCLE_LIMIT`, `RATE_LIMIT_EXCEEDED` ⇒ `queued`, retry later. `PKI_SEND_FAIL_PUBLIC_KEY`, `PKI_UNKNOWN_PUBKEY` ⇒ `queued`, `key_unknown`; the
  bridge never asks a node for its key (not on the allowlist), so the DM resumes when that node's NodeInfo with a key is heard, or by hand.
- **Receive.** Portnum 1 becomes a text; `emoji != 0` with `reply_id` is a tapback, not a message. `via_mqtt` is kept in `meta` and shown as "via
  internet". Channel text is `channel-key`; a DM to the board (`to == my_node_num`, `pki_encrypted`) is `pki`. A non-PKI DM to the board is dropped by
  the firmware ("legacy DM"); the DM copy says so. Texts received while no client reads can be lost (the board's phone queue holds 8–32 packets, R01
  §3.11); the banner says so.
- **Channels and names.** All configured channels appear read-only from first run; sending needs the channel chosen; relaying the default public
  channel needs an explicit edge (§8). The firmware rewrites `from`: the bridge speaks as the board's own node (R01 §3.3). 111 reads and shows its
  `long_name` (24 B limit) and **never writes node configuration**; D17's `meshtastic.set` stays reserved (§12). **A DM to the board's node lands in
  this node's Messages** (its paired humans; hub-member routing waits for multi-device). A foreign DM is opened from Nearby or a message ("Send a
  Meshtastic DM from this node (readable, not 111-sealed)"), never as a contact. **DMs are PKI-only** (R01 §3.6): the destination's key must be in the
  board's NodeDB; DM cap 220 B, or 232 B on a licensed board where the firmware skips PKI.
- **HAM and MQTT: live consent (bridge M3).** The client keeps `User.is_licensed` and each channel's `uplink_enabled` current. On a licensed board,
  readable sends and relays through that account stay **off until a human confirms** once per account; sends on an uplinked channel are **off by
  default**, one switch per channel, with "this channel is published to the internet by your board". Both confirmations use the text-plus-hash record
  of §8, not a bare `PATCH`; the licensee's content and third-party-traffic rules are the licensee's fact (DEV §0.6); sealed frames never. **The
  uplink switch gates every send on that channel, relay copies included**, and the facts are read per copy, so a board that turns uplink on or becomes
  licensed after enable pauses the edge (§8). Our packets are uplinked whenever the channel uplinks, whatever `ok_to_mqtt` says (R01 §3.12); a DM can
  go out as an opaque blob when any channel uplinks, and the DM compose warns once.
- **Service-page copy (replaces L5 §14 "111 never transmits"; R01 §4):** "111 transmits on Meshtastic only what you send or choose to bridge, as a
  normal text from your board's own node `<long name>` (`!<id>`). Anyone on that channel can read it: Meshtastic protects channel messages with the
  channel's key, and the default channel's key is public. If your board uploads to an MQTT server, messages on channels with uplink enabled are
  published there. Messages from Meshtastic that appear in 111 are not 111-sealed and are labelled with their Meshtastic sender. Meshtastic is a
  registered mark, used nominatively." What the user does in the panel is not seen by Messages: the firmware does not echo phone-sent text to other
  clients (R01 §3.19, unverified on hardware).
- **Nearby folds into the client**, which publishes the same `NearbySource` JSON (`kind: meshtastic`). The Python reader and its file ingest
  (`N/meshtastic/presence.rs`) retire **after a parity gate**: one committed `FromRadio` fixture goes to both readers and each field of the
  `meshtastic.json` schema (L5 §14: `id`, `num`, `long_name`, `short_name`, `pubkey`, `snr`, `rssi`, `hops_away`, `last_heard_hint`, `position`,
  `channel_utilization`) must match, names compared cut to 24 B because the reader truncates at 40 B and the protobuf limit is 24 (R01 §3.3).
- **The shim drops `ToRadio.disconnect`:** `shim.rs` checks one tag byte (field 4, `0x20`) on `PUT /toradio` and discards it, still without decoding,
  because a raw disconnect would close the board's session for every client (R01 §3.19).
- **Versions.** Tested on 2.7.26; 2.8.1 (key-derived node numbers, XEdDSA, new region table) is a bench item.

## 6. LXMF adapter: our own helper (normative; D26 #4)

**Adopt, don't reinvent:** Reticulum and LXMF are used as released: `rns` and `lxmf` from PyPI, unmodified, **downloaded at install time, never
bundled** (D26 #5). The carrier is `lxmf-sidecar` (`6-transport/sidecars/lxmf-sidecar/`), modelled on the presence sidecar
(`5-messaging/sidecars/meshtastic-presence/`): one Python file of a few hundred lines, `requirements.in`, a lock from `gen_lock.py`, `run-tests.sh`
and offline `tests/`, a README and a LICENSE. It runs in its own venv `venvs/lxmf`, built by the L1 pattern (`pip install --require-hashes` from the
lock the release carries, closure cached under `cache/packages/lxmf/`, §2.1) on the distro `python3` (`rns` and `lxmf` need only Python ≥ 3.7, R02
§2). Its lock pins `rns` (and `lxmf`'s `rns` requirement) to the same version as `rns-lock.txt` at release time, because two `rns` copies share one
instance protocol; `gen-notices.py --check` or the release checklist asserts that the two locks agree. It is the one program of ours that imports
Reticulum, a separate process never combined with `111d` (D17 #12 amended, §15, §18). **Credit:** its header, README, `THIRD-PARTY-NOTICES.md` and the
Settings copy say "Reticulum and LXMF are the work of the Reticulum project (reticulum.network); 111 uses them as released and is not affiliated". Its
licence is **pending the owner's contact with upstream** (§15). Evidence: the Reticulum side is *run* (R07 checks 1, 1b, 7); the helper is *design*
until G3 (R02 §4 C, *src*).

**Process contract.** A `ChildSpec` with piped stdio (§7), spawned only while the Reticulum bridge is enabled: `python3 -I lxmf_sidecar.py
--config-dir <bridge/lxmf/rns> --identity <bridge/lxmf/identity> --storage <bridge/lxmf/store> --name <display name>`, `HOME=<root>`. **JSON Lines,
UTF-8, one object per line, at most 64 KiB; stdout is the protocol, stderr the log.** Piped stdio and not the presence sidecar's snapshot file because
the helper is bidirectional (send, announce, name), listens on no port (§15.7), and the `Piped` extension is needed by `deltachat-rpc-server` anyway
(§7). `111d` validates every line (size, schema, field caps) and counts a malformed one as noise (draft proposal for the messages):
- `111d → helper`: `send {id, to, text, fields?}` · `announce {}` · `name {display}` · `quit {}`.
- `helper → 111d`: `ready {dest, pubkey, instance}` (`dest` = the LXMF delivery destination hash the library reports, `pubkey` = the identity's 64 B
  public key, so G3 can exercise the Wave 0 verifier's hash binding end to end, §14) · `msg {hash, src, name, text, claim_ts, signed, why_unsigned,
  fields}` · `state {id, hash, state, reason}` · `heard {dest, name}` (an announce: feeds Nearby `reticulum` sources and re-queues a
  `peer_unannounced` send) · `link {up | down | partitioned}`.

**What the helper does.** (1) Attaches to the shared rnsd instance as an interface-less client: its config dir has `share_instance = yes`,
`instance_name = 111` and rnsd's `rpc_key` (a different dir derives a different key, R02 §3.4), dir 0700, `config` 0600 (§11); *(unverified)* if the
installed RNS has a require-shared-instance mode the helper uses it (confirmed at build); the start order and the post-spawn `rnstatus` check below
are the mechanism. (2) Owns the delivery identity `bridge/lxmf/identity` (0600) and one destination; `111d` does not read the file in R-B (§14). (3)
Announces on enable, on every `link up`, on request and on a user-set interval (default 60 minutes, `0` = events only; Reticulum caps announces at 2 %
airtime per interface); display name default `111-<4 hex of the address>`; **no stamp cost** advertised (no policy), the router pays others' (R02
§3.12). (4) Sends with the library's `LXMessage` (it picks opportunistic, direct or link) and **never auto-resends**. After the library's 5 attempts
(R02 §3.10): a `FAILED` that came after `SENDING`/`SENT` was on air without proof, so the row keeps `sent` with "transmitted, not confirmed" and is
`Retry::Later` on that peer's next `heard`; a `FAILED` while still `OUTBOUND` (no path, nothing sent) is `queued` with `peer_unannounced`; only
`REJECTED` (stamp, blackhole) and the user's `CANCELLED` are `failed` (bridge M2). (5) Receives through the delivery callback; the **message hash is
the dedupe key** (R07 §2 check 4); **`signed` is the library's `signature_validated`**, with `unverified_reason` otherwise (R02 §3.6, §4 C, *src*);
attachments are not ingested. (6) Reports states monotonically: `OUTBOUND` queued, `SENDING`/`SENT` sent, `DELIVERED` delivered, a propagated `SENT`
relayed, `REJECTED` and a user `CANCELLED` failed, a `FAILED` as in (4). While it is down the node receives no LXMF (a propagation node would hold it;
opt-in, off).

**Relay marker and fields.** The visible `[name/TAG] ` prefix (§8) leads the text, and a structured copy rides the LXMF fields **the library lets a
sender set**: any field id (the `fields` dict is serialised as given), title and content (R02 §3.13, §4 C, *src*). The helper uses `FIELD_CUSTOM_TYPE
= "111.relay"` and `FIELD_CUSTOM_DATA = {v, net, id, mid}`, the ids LXMF reserves "for bridging/tunneling external protocols". Stock apps ignore
custom fields, hence the prefix. R07's field-16 finding belonged to MeshChatX's allow-list and is dropped; G3 asserts the custom-field round trip.

**Start order (R07 check 1b, *run* for the trap).** `rnsd` must own the shared-instance socket before the helper starts, or the helper becomes the
instance and the node partitions silently. The component runner (§2) spawns the helper only while the supervised rnsd is `Running` and `rnstatus`
(adopted, from `venvs/rns`) shows the instance serving, stops it before any rnsd restart and starts it after, and after **every** helper spawn, the
supervisor's back-off respawns included, checks that `rnstatus` still shows the instance serving; otherwise it stops the helper, reports
`adapter_down` ("partitioned") and retries once rnsd serves. G4 asserts the restart path.

**rnsd posture v2** (D17 #5, rendered from `interfaces.toml`, §11): `share_instance = yes` (Reticulum's default; `config.tmpl:9` says `no` today) ·
`instance_name = 111` · `rpc_key` from `secrets/rns-rpc-key` · `enable_transport = yes` on Cat 3/4 (the phone-relay role needs the hub to route; R02
argued an endpoint needs none, D17 #5 stands), `no` elsewhere · the `111.network` AutoInterface kept · a `BackboneInterface` listener on the LAN only
while LAN sharing is on (IFAC optional, passphrase in `secrets/`, shown only on a paired or loopback session) · a second AutoInterface in the stock
`reticulum` group and the propagation-node role are opt-ins, **off by default** (two AutoInterface blocks share default ports: *unverified*, a G4
case) · interface changes use `rnstatus --attach|--detach|--reload`, never `pkill` (`N/transport/mod.rs:178` today).

**"Send to Reticulum".** With no channels a target is a **chosen peer address**: per-network recipient lists (Settings or the compose picker; 32-hex
addresses entered, pasted or picked from heard announces in Nearby), one LXMF DM per recipient. A recipient list is a destination-only `group` (§8). A
distribution-group hub and Reticulum Relay Chat rooms are deferred (§17). The native UIs for Reticulum users are Sideband and Columba on a phone;
MeshChatX may return as a hosted panel (§17).

## 7. App-peer adapters (normative shape; Delta Chat is the reference)

An **app peer** is a hub-side headless peer of a phone messenger. P2 as worded (the agent installs and drives apps on the phone) is not feasible:
Termux:API cannot drive another app's UI or install an APK silently, and same-phone ADB is a human-paired, fragile session (R03 §3.1–§3.5). Evidence
here is *doc* only: no phone and no app was run.

**Shape, for every app peer.** (1) One headless peer process, an optional component (§2.1), owning a dedicated **bridge account**, never the user's
own. (2) **A warning once per account, then a per-contact opt-in.** At enable, verbatim: "This hub decrypts everything this account receives,
including messages from contacts you do not bridge. The plaintext exists on this hub." Bridging is off for every contact until the human turns it on
for that one ("This hub decrypts this contact's messages to bridge them. What you relay onto another network is readable there."). The core stores
every contact's decrypted messages regardless, so 111 sets the core's `delete_device_after` for non-bridged chats where it offers one, or documents
the residual; the opt-in decides only what 111 copies into Messages, shows and relays. The app's store is named in §15.2 and removed by "remove
files". (3) The bridge account is flagged as a bot where the protocol allows, bridged copies carry the relay marker (§8), and the banner says the hub
terminates the app's end-to-end encryption. (4) Kinds `dm` and `group`; only groups can be relay endpoints. (5) The app's own UI is offered separately
(below). (6) Public servers (chatmail relays, SimpleX presets) are **off by default**; a per-adapter switch enables them with the operator terms
shown.

**Spike first (D26 #6).** A netns spike precedes any Delta Chat build row: `deltachat-rpc-server` plus Madmail in IP-only mode, two accounts exchange
a message over a veth, and ports and the unit's capability are pinned from what runs. If it fails, **Delta Chat leaves R-B** and returns with the
phone bench. Whether the mobile apps accept an IP-only, self-signed relay stays an attended hardware gate (§16). This section is the normative shape;
the Delta Chat adapter is "built after the spike passes".

**Delta Chat (reference).** `deltachat-rpc-server` (chatmail core, MPL-2.0, Rust; linux x86_64/aarch64 and an Android arm64-v8a build, so a Termux
phone could later run the adapter: a runbook gate, not a v1 claim) speaks JSON-RPC 2.0 over **stdio**: no port (R03 §3.9). The bridge account sets the
core's bot flag (`Config::Bot`). Contacts join by Secure-Join: the hub shows the bridge account's QR, which is also the contact's opt-in act. States
map as in §4 (*doc*). The relay is **Madmail** (Rust single-binary chatmail relay, AGPL-3.0, IP-only and self-signed mode, experimental), LAN-bound
only while LAN sharing is on, addresses like `user@[192.168.1.5]`; its licence file is read before its pin (§15).

**Supervisor extension (needed; shared with the LXMF helper).** A `ChildSpec` (`N/supervise.rs:55-64`) gains `stdio: Null | Piped`. Today every spawn
sets `stdin = null` (`:449`) and sends stdout and stderr to a log or inherits them (`:450-481`). For `Piped` the supervisor **owns a reader task for
stdout** (an undrained pipe blocks the child and the respawn loop), exposes the lines through a bounded channel on its `ChildControl` (`:155`) with a
stdin writer, and re-publishes both after every respawn; the adapter re-attaches, requests in flight fail with `adapter_down` and re-queue (§4);
stderr still goes to the log. A tiny socket wrapper is the fallback. **(draft proposal)** for the names.

**SimpleX (second; glibc hubs only).** The SimpleX CLI (AGPL-3.0, Haskell, Ubuntu-glibc binaries, dynamic linking unverified) runs as a local
WebSocket server (loopback 11232) with a documented bot API (`peerType: "bot"`), and `smp-server init -y -l --ip <LAN address>` is the documented
offline server (§2; R03 §3.11). Separate processes (AGPL). The component **refuses to enable on Termux**, the only non-glibc host, with honest copy.
Built after Delta Chat; whether it ships is the owner's hand-selection.

**Briar: online-only, documented, not built.** `briar-headless` registers only the Tor plugin on Linux (R03 §3.8); offline transports exist only in
the Android app and Briar Desktop, neither with an API. The KB entry says so (§18).

**The native UI, offered separately (D25 F4).** An app with a web UI would be a hosted panel (not built in R-B, §17). None of the three has one, so
each gets a **link-out card**: "Open Delta Chat on your phone or desktop", the bridge account's QR, and "this is the native app; Messages shows the
same conversations as text". **Driving a phone's UI** (wireless ADB) stays a user-attended runbook outside v1; a phone action, if ever offered, is a
typed `111d` action behind human approval, never an agent shell.

## 8. Relay edges, loop guard, consent (normative)

An **edge** is an explicit, directed, non-transitive relay from one conversation to another: A to B plus B to C never gives A to C; a bidirectional
pair is two edges. A relayed copy is a new `own` row on the destination adapter and follows the §4 states; the origin sender gets no relay receipt.

- **Endpoints.** Both `channel` or `group`; a DM is unrepresentable (the API refuses it with `400`, the `src_kind`/`dst_kind` `CHECK` backs it, §3);
  `src ≠ dst`; an edge never carries 111 sealed threads. **LXMF has no channels**, so a Reticulum **recipient list** (a local `group`-kind object, §6)
  can be an edge **destination** only (one LXMF DM per member from the bridge identity, each with the marker); there is no Reticulum source
  conversation to read, so relays *out of* Reticulum do not exist in v1 (draft proposal; §17).
- **Who enables.** Only a human on a **paired or loopback session** (`API:47`); the agent's bearer is refused by the global write gate (§13). The
  agent files `bridge.propose_edge` (proposal `kind: bridge_edge`, body `from: <fc-id>` and `to: <fc-id>` lines, draft proposal) and the human
  accepts. `POST /bridge/edges` without `accept_warning` answers `409 consent_required` with the exact warning text; the client echoes its hash
  (`consent_hash`). **Accepting a `bridge_edge` proposal runs the same round-trip** (`decideProposal` answers the 409 with the text, the dialog shows
  it, the decision carries the hash). The warning: "Everyone in `<src>` will have their messages copied to `<dst>`. They see the `[name/TAG]` prefix
  only if they read `<dst>`." Add-ons when true: the source is a default-key channel ("anyone can read it; this copies public traffic"); the
  destination is on an MQTT-uplinked channel; the destination account is licensed (third-party traffic is the licensee's fact).
- **Consent facts are read live, per copy (bridge M3).** Uplink, licensed mode and the channel key can change on the board after enable. For every
  copy the relay re-evaluates the add-ons; if the set differs from the consented one, the edge is **paused** (`consent_paused`) with the new text and
  resumes only on a new human confirmation. It never relays on with a flag.
- **What it relays.** Inbound rows with `cursor_seq < local_seq` (set at enable: **history is never relayed**) and `local_seq > relayed_seq`
  (persisted per edge, advanced on a handoff **and on a shed**, so shedding never makes history relayable again, bridge M6) that are not relayed
  copies and do not parse as already relayed. A copy longer than the destination's cap is never truncated: per-edge `oversize` is `refuse` (default;
  counted as "N too long to relay") or `split` (numbered parts).
- **The marker (normative; bridge B1).** The prefix is `[<n>/<TAG>] `. `<n>` is the origin sender's name after the §3 ingest function, reduced to the
  marker alphabet: no `]`, `/`, Cc or Cf, cut to 10 bytes at a UTF-8 boundary, `?` when nothing is left (`W1AW/7` becomes `W1AW7`). `<TAG>` is one to
  three ASCII letters registered per adapter manifest (`M`, `LX`, `DC`, `SX`; draft proposal for the last two). **The marker leads every copy and
  every part of a split copy** (`[n/TAG] (k/n) text`) and is counted in the preview. The parser accepts an optional `(k/n) ` before it, so a part
  number added by another tool cannot hide a marker, then `[`, one to ten bytes without `]`, `/`, Cc or Cf, `/`, a registered TAG, `]` and a space.
  Any 111 node treats a text that parses as already relayed and relays it nowhere, which stops loops between nodes; a human who types a marker gets no
  relay, and the copy says so. LXMF also carries the custom field (§6).
- **Guards, all required** (R04 §4.4): (1) **never re-relay a relayed copy**: rows with `origin_msg` or `edge_id` are never edge inputs · (2)
  **own-echo suppression**: our sends are recorded with the native id (Meshtastic id from `QueueStatus.mesh_packet_id`; LXMF hash); inbound from **any
  account of this node** (two boards of ours hear each other), from our LXMF destination, or matching a recorded id is dropped as inbound (whether the
  firmware ever returns our own rebroadcast is *unverified*; the guard assumes it can) · (3) **the marker** · (4) foreign bridges (mmrelay,
  matterbridge shapes) only by an optional "looks relayed" hint, default off · (5) **two 111 bridges on one pair** duplicate but cannot loop, and
  nothing warns in R-B ("already bridged by node X" needs attestations, §17) · (6) **the ID map is `foreign_seen`** (bounded keys per edge, §3) plus
  `origin_msg`/`edge_id` on live rows; bodies are prunable and `relayed_seq` is the high-water mark.
- **Defaults accepted in D25 F7:** no courtesy notice on the source channel (airtime; not offered) · readable relay into an amateur-licensed radio
  only after a human confirms, sealed frames never · a backlog is held to capacity, then shed oldest with a visible counter on the edge (§3 step 2b).
  **Not a default:** honouring `ok_to_mqtt` as a relay filter; it is the per-edge switch `honour_ok_to_mqtt`, **off by default**. When on, a
  Meshtastic-origin message whose `Data.bitfield` lacks the ok-to-MQTT bit is not relayed off the mesh and `refused` counts it (the bit position is
  read from the pinned protobufs at build; R04 §3.9 marks it unverified).
- **Airtime honesty.** 111 sets no rate, size or expiry limit. It shows facts: per-send airtime, the region's duty-cap, the edge's backlog ("waiting
  for the radio" when `QueueStatus.free = 0`), firmware refusals with reasons. An edge's live counters let the agent suggest turning it off; it cannot
  do so.

## 9. Compose fan-out (normative)

A 111 message enters a foreign network only by the user's explicit act. The 111 thread composer is unchanged; fan-out is a separate compose ("Send to
networks…") whose result is a **copy of the text** sent as a fresh send group, never a 111 message exported.

- **Targets.** Any conversation of any enabled network, the per-network recipient lists (§6), and 111 contacts, held in one send group
  (`send_groups.targets`: 111 targets stay ordinary sealed messages with their L5 states, foreign targets are `foreign_messages` rows).
- **Preview** (`POST /messages/fanout/preview`; no queue write, no transmit, no log of the text), per target: UTF-8 bytes; the target's
  `max_text_bytes` for its kind and mode; parts if split; **airtime estimate** from the adapter's profile (Meshtastic: the time-on-air formula for the
  board's `modem_preset`, KB `lora-airtime-budget`, "255 B on LongFast is about 2.1 s", labelled an estimate; LXMF: "depends on the path"); region and
  TX status (`region_unset`, duty-cap fact, `ham_unconfirmed`, uplinked-channel warning, adapter down); refusals ("312 B > 232 B per packet").
- **Size modes per target.** `refuse` (default) or `split` (explicit per send). Split cuts at UTF-8 boundaries, prefers whitespace, numbers parts
  `(1/3) ` and counts that overhead; Meshtastic parts can arrive out of order and the preview says so. **Truncation never happens silently.** A split
  target is **one `foreign_messages` row per part**; the UI shows the lowest state among its parts, and cancel drops unsent parts only (bridge minor
  9).
- **Send** (`POST /messages/fanout` → `{group_id, items: [{target, id, plan}]}`). Validation is atomic: a target refused for `oversize` in `refuse`
  mode or for a consent gate (`ham_unconfirmed`, `uplink_off`) answers `409` with the per-target report and queues nothing. Adapter *state* (region
  unset, board away, queue full, no path) is not a call error: rows queue with the reason, because delivery is non-negotiable. Each row has its own
  state, retry and cancel (only while `queued` with nothing sent, as `Fabric::cancel`, `M/fabric.rs:794`); one target's failure never cancels another;
  pacing is the adapter's.
- **Agent.** `messages.send` gains optional `targets` and stays a proposal: `kind: bridge_message`, body = `to: <fc-id>` lines, a blank line, the text
  verbatim (draft proposal; R04 §4.8), rendered **preformatted, never as Markdown** (§10). Accepting runs the same validation and, if a consent gate
  applies, the same 409 round-trip; a refusal leaves the proposal open with the problem.

## 10. Messages view, contract and SSE (additive; binding form is `4-interface/api/openapi.yaml`)

**Rail.** "111" first, then one entry per enabled network in the user's preferred order (§11), each split into channels and DMs; a thread view per
conversation; each foreign message shows the network chip, the native id (`!a1b2c3d4` or the hash), the untrusted name as text and the `auth` label
with its copy; each banner shows the adapter's honest copy and the dropped-text counter (§4). **Attention (lead reading of bridge minor 2):** a
foreign DM to the board raises the **unread count only**, never the header badge (`UI/lib/attention.ts` counts proposals, errors and message requests,
not unread 111 messages); channel traffic raises nothing unless pinned. Compose uses a **foreign compose gate** (network up, region set, uplink
warning, HAM confirmation, airtime fact) instead of the contact-based `composeGate` (`UI/views/messages/model.ts:218`). Nearby and a foreign sender
get "Send a Meshtastic DM from this node (readable, not 111-sealed)", which opens a foreign DM. Labels come from the adapter manifest.

| Route | Verb | Purpose |
|---|---|---|
| `/networks` | GET | `{schema, networks: [Network]}`: id, label, chip, state `live · not_set · error · disabled`, account, copy, caps, counts, dropped-text counter, component state with cached and installed size |
| `/networks/{net}/conversations` · `/{conv}` | GET | conversations; one conversation with `?limit&since_seq` messages |
| `/networks/{net}/conversations/{conv}/messages` | POST | `{text, size_mode?}` → 202 `{id, plan}` |
| `/messages/fanout/preview` · `/messages/fanout` | POST | §9 |
| `/bridge/edges` · `/bridge/edges/{id}` | GET · POST · DELETE | §8; `409 consent_required` carries the warning |
| `/networks/{net}/conversations` | POST | open a foreign DM (`{kind: "dm", native_id}`) from Nearby (draft proposal) |
| `/bridge/settings` · `/bridge/components` | GET · PATCH | adapters on/off, preferred order, recipient lists, per-channel send switch, component enable (with the licence acknowledgement) / disable / remove files (draft proposal) |
| `/transport` · `/transport/posture` · `/transport/join` | GET · PATCH · POST | `transport.read` facts; posture through the applier (§12); the phone-relay join descriptor, a POST because it carries credentials and the bearer reads every GET (draft proposal) |

**Every write above refuses the agent's bearer** through the global gate (§13); G1 asserts `401` on each. HAM, uplink and component confirmations use
the 409 round-trip with the text hash, not a bare `PATCH`.

**Schemas (new):** `Network`, `ForeignConversation` (`fc-…`), `ForeignMessage` (`fm-…`: `dir`, `sender {native, name}`, `auth`, `text`, `state`,
`evidence`, `reason_code`, `group_id`, `part`, `relayed_from`, `via_internet`, `seq`, `at`, `claim_ts`, `truncated`), `SendGroup`, `BridgeEdge`, and
the closed enum `ForeignMessageState`. **SSE (same `/events`, additive):** `network_message` `{network, conv, id, change}`, coalesced per message per
tick for DMs and own rows and **per conversation per tick** (conversation plus highest seq) for unpinned channel traffic, so distinct messages in a
flood cannot fill the 256-entry ring; `bridge` (adapter and edge status). The UI store ignores unknown events (`UI/lib/api/store.ts:375`, a `switch`
with no default).

**Enum growth and the pins each bump touches.** `Proposal.kind` + `bridge_message`, `bridge_edge`, `device` (`N/dashboard/api_v1/proposals.rs:65`
`KINDS` 5 → 8, `API:556`) · `NearbySource.kind` + `reticulum` (`API:920`; sources are announces the helper hears) · new `ForeignMessageState`
(`API:687` `MessageState` stays closed at four) · `reason_code` per §4 · `Problem.code` + `consent_required`, `oversize`, `component_unavailable`,
`unsupported_action` · agent tools 10 → 17 (`A/tests/daemon_tools.rs:306`, `A/tests/chat_loop.rs:308`) · regenerate with `pnpm gen:types`, run
`gen-sse.mjs --check` (`4-interface/ui/package.json:14`). `ServiceId` does **not** grow: R-B hosts no panel, and the `transport` row already reads
"Reticulum". Unknown values render as `unknown` (U1b rule).

**Proposal bodies are text, not Markdown.** The agent view parses proposal bodies as Markdown with external links
(`UI/views/agent/proposal-body.tsx:5,9`), so a stranger's text quoted into a proposal could render as a link. For `bridge_message` and `bridge_edge`
the contract says the text part renders as a preformatted block (bridge M8).

**UI.** `/messages/net/:network[/:conv]` is placed **before** `/messages/:peer` (`UI/app/routes.tsx:39-41`, the `/messages/contacts` rule); the view
is a lazy chunk under D21's 64 KB cap. `/settings/l6` becomes real (`UI/views/settings/index.tsx:15`; the `l6` slug exists, `UI/lib/layers.ts:55`):
adapters and their order, rnsd posture, region and callsign (D17 #10), edges, recipient lists, optional components with sizes and licence notice, HAM
and uplink switches.

## 11. Transport spine (normative)

**`state/interfaces.toml`** is the config home (D17 #6) and the **single source of truth** for interface ownership: `111d` renders from it, the agent
proposes typed actions, the human decides. 0600, written atomically; it inherits the Meshtastic binding from `devices.json`. Shape **(draft proposal;
normative once accepted)**:

```toml
schema = 1
[radio]                           # D17 #10: asked once at first radio setup; the agent suggests, never sets
region = "EU_868"                 # id from the KB region table; absent until a human sets it
callsign = ""                     # optional; licensed operators
[posture]                         # D17 #5; Settings → Layer 6 is the user-facing setting
share_instance = true
instance_name = "111"
enable_transport = true           # default: true on Cat 3/4, false elsewhere
[posture.lan]                     # rendered only while LAN sharing is on
backbone = { listen = true, port = 4242, ifac = false }
stock_autointerface = false       # second AutoInterface, group "reticulum": opt-in
propagation_node = false          # LXMF propagation role: opt-in
[order]                           # README §6: the preferred networks, in the user's order (network ids, §3)
networks = ["111", "lxmf", "meshtastic", "app:deltachat", "app:simplex"]
[[interface]]
name = "RNode 1"
owner = "rnsd"                    # 111d | rnsd (D19 c): one owner per port
type = "RNodeInterface"           # rendered stanza; params below
device = "<DeviceRecord id>"      # stable id (DEV §7.1)
port = "/dev/serial/by-id/usb-…"
region = "EU_868"                 # optional per-device override of [radio] (D17 #10)
params = { frequency = 868100000, bandwidth = 125000, txpower = 14, spreadingfactor = 8, codingrate = 5 }
[[interface]]
name = "Bench radio"
owner = "111d"                    # a board 111d drives itself (Meshtastic via meshmud; the 111 modem in the second release)
type = "meshtastic"
uplink = "/dev/serial/by-id/usb-…"
```

- **Order (README §6).** The "short, ordered list of preferred networks, curated in the Knowledge Layer" is a KB concept entry
  (`concepts/preferred-networks`, candidate; default 111 fabric, Reticulum, Meshtastic, app peers) plus `[order]`; Reticulum is the transport under
  `lxmf` and has no entry of its own. In v1 the order drives the agent's setup proposals, the Messages rail and Settings. **The runtime behaviour
  (join in sequence, heartbeats, fall back through the list) is Layer 7's presence and gateway work, deferred explicitly;** until then the node is on
  every enabled network at once.
- **Migration (integration M7).** On the first start with Layer 6, if the file is absent, the Meshtastic binding is copied from `devices.json`
  (`N/devices/store.rs:141`) as an `owner = "111d"` entry and every enabled RNode stanza in `rns/config` (`N/devices/rns.rs`) becomes an `owner =
  "rnsd"` entry. From then on **`interfaces.toml` wins**: the Layer 5 readers of the binding (`N/meshtastic/mod.rs:564`, `N/devices/store.rs:256`) are
  switched to it, the copy back into `devices.json` is **write-only** (kept for a Layer 5 rollback, never read), and a divergence at start is resolved
  for the TOML and logged. The default posture moves from `share_instance = no` to `yes`, so the first start after upgrade restarts rnsd once; the
  release notes say so.
- **Renderer v2** (`N/transport/config.rs`). `SCHEMA_VERSION` 1 → 2 (`:11`); `render()` (`:22`) takes the parsed TOML; the template keeps
  `schema-version` first. **Modes (carriers M1):** `write_if_changed` writes `rns/config` with `0o644` today (`:39`) and v2 puts the `rpc_key` there,
  which since RNS 1.5.5 authorises `rnstatus --attach|--detach` over the control RPC; so v2 writes `rns/config` **0600**, `rns/` and the helper's
  client dir **0700**, and G4 asserts the modes. v1 binaries already refuse to regenerate a newer file (`N/transport/mod.rs:149-153,170-171`), so
  rollback is safe. Stanza names are sanitized and `port` is validated as in L5 §14. With no region set, RNode stanza proposals are refused (transmit
  legality is a per-node human fact, DEV §0.6); the frequency plan comes from the KB region table, generated from the pinned `config.proto` (R05
  §3.16). rnsd has no validate flag, so validation is **start rnsd on a scratch `--config` dir and read `rnstatus`** (§19).
- **Live attach, detach, reload.** `rnstatus --attach|--detach|--reload <name>` over rnsd's control RPC (RNS ≥ 1.5.5; *run* against 1.5.7, R05 §3.9).
  Posture keys need a restart through `ChildControl::restart` (`N/supervise.rs:324`) with the child ordering of §2 and §6. The netlink path, which
  today regenerates the config and `pkill`s rnsd on every link change (`N/transport/mod.rs:165-186`), becomes: re-render, diff, `--reload` the
  AutoInterface, restart only for posture.
- **`transport.read`.** Interface facts `{name, type, up, rx, tx, peers}` from `rnstatus` through the shared instance, replacing the "joined"
  heuristic that scans `rnsd.log` on a wall-clock window (`N/transport/mod.rs:190`). `rnstatus` is a Python process, so it runs **on a tick into a
  cache**; the route and the tool read the cache, never the request path. Golden-file parser tests with a fake `rnstatus`; `-j` is used once its
  schema is captured (*unverified*).
- **Phone relay v1 (D17 #4, TCP to hub first).** The hub, with LAN sharing on, renders a `BackboneInterface` listener on 4242 (Linux and Android,
  interchangeable with TCP server/client, auto-blocks flapping clients, R05 §3.12), IFAC optional; the Termux node's rnsd gets a `TCPClientInterface`
  to it: unicast, no MulticastLock. **QR join** reuses the Layer 4 pair-code ceremony that L5 §7.3 reuses: the hub shows a QR with its address, port
  and a one-time pair code on a paired or loopback session; after pairing the phone's `111d` fetches a join descriptor (address, port, IFAC name and
  passphrase if set) over the paired session (`POST /transport/join`, draft proposal) and writes its own entry. The passphrase never rides in the QR.
  Evidence: Backbone plus TCPClient in two netns reached "Up, Clients: 1" (*run*, R05 §4.1); the phone side is a hardware gate. Hotspot, MulticastLock
  AutoInterface and BLE stay out. Columba is a KB candidate; the manifest links to it and ships no APK.

## 12. Device core (normative)

- **Hotplug.** A `HotplugSource` trait (injectable, like `LinkEvents`, `N/transport/netlink.rs:1-30`) with a production source on
  `NETLINK_KOBJECT_UEVENT` via libc (already a dependency, `Cargo.toml:36`; group 1 needs no privilege) kicks the registry tick; the 5 s poll
  (`N/devices/registry.rs:75`, `N/devices/mod.rs:331`) stays the authoritative fallback, the 2-tick debounce (`registry.rs:77`) unchanged. If the unit
  restricts address families it must keep `AF_NETLINK` (§18). The registry reports `port_busy` naming the holder when ModemManager or brltty has the
  tty open (DEV §8); 111 never kills them.
- **Port ownership.** The `PortTable` (`N/ports.rs:106,125,146,165,190`) is the one authority for radio ports; `rnsd` and `111d` claim there too,
  before a stanza is attached. Ownership is no longer inferred from config stanzas (`N/devices/registry.rs:1458-1467`); it comes from
  `interfaces.toml` and the claims.
- **Identify, once per device, consented (D17 #8), through adopted tools.** Orchestration moves from `N/meshtastic/identify.rs` to
  `N/devices/identify/`; steps in DEV §3.2 order, one probe per 30 s per device, each with a timeout: (1) **RNode:** `rnodeconf --info <port>` from
  `venvs/rns`, parsed against a **fake `rnodeconf` script** (no stub is modelled on Reticulum source, R06 G6) · (2) **Meshtastic:** meshmud with the
  identify profile, as built · (3) **MeshCore:** the documented MIT `GetVersion`/`GetIdentity` exchange (`docs/kiss_modem_protocol.md`, R05 §3.1);
  companion-firmware identify is deferred. Each step returns `Match`, `NoMatch` or `Error`; a missing tool is `Error`, never `NoMatch`. `family:
  unknown` is persisted only when **all** steps answer `NoMatch` (today exit code 3 persists it after the Meshtastic step alone,
  `N/meshtastic/identify.rs:23-25`). The offer appears whenever any step can run, not only when meshmud and the reader are installed
  (`identify.rs:200-202`). The port is released at the end or its claim transferred (`PortTable::transfer`). Over-matching `lora-table.toml` rows
  (`10c4:ea60`, `1a86:*`, `303a:*`) stay a `likely` hint and are refined here.
- **Proposals.** `device.propose` (§13) files `kind: device` with actions from the closed vocabulary (DEV §3); the UI renders a device card with the
  exact stanza and port (a restart proposal renders a **service card**, not a device card). `kind: device` also carries the two one-shot actions
  below, so **no new proposal kind is needed for identify or restarts**.
- **The narrow apply→verify→undo cut (P4, D17 #7).** Four actions with a revert: `rns.interface.upsert`, `rns.interface.remove`, `rns.posture.set`,
  `radio.region.set`. Two one-shot actions with nothing to revert: `identify {device_id}` (the per-device consent, remembered on accept) and
  `service.restart {service_id}` (supervised children only, `409 not_supervised` otherwise, `API:47`). Everything else in the vocabulary
  (`meshtastic.set`, `ble.pair`, `ble.forget`, `firmware.flash`) is refused with `unsupported_action` (new `Problem.code`), never best-effort. Loop:
  **snapshot** `interfaces.toml` and the rendered config → **apply** → **probe** → **keep or revert**, the result written to the proposal; a crash
  mid-apply leaves a visibly half-applied proposal, never a second apply (the L5 `applied: {pending: true}` pattern, L5 §15).

  | Action | Apply | Probe, within 30 s (monotonic) |
  |---|---|---|
  | `rns.interface.upsert` | render, `rnstatus --attach` | interface present in `rnstatus` and up (an RNode that cannot open shows down while rnsd runs, R05 §3.10) ⇒ else revert (`--detach`, restore) |
  | `rns.interface.remove` | `rnstatus --detach` | absent from `rnstatus`; revert re-attaches |
  | `rns.posture.set` | render, stop the helper, restart rnsd, start the helper | instance up and the helper re-attached; else restore and restart again |
  | `radio.region.set` | write `[radio]` or the device's `region` | file re-parses; no radio effect by itself. Only a user-confirmed region: acceptance is the confirmation |
- **Flashing stays contract-only until the bench** (`/devices/{id}/flash` has no route, `N/dashboard/api_v1/devices.rs:181-186`, `API:85`; no
  `device.flash` tool). The MeshCore KISS image has no upstream CI build (R05 §3.3), so D17 #9's CI-built, manifest-pinned image belongs to the second
  release. `rnodeconf` stays the only path for RNode provisioning and flashing. **Meshtastic node configuration is never written in v1**; region, name
  and preset are set in the panel.

## 13. Agent tools (Layer 3 set grows; fixed, shell-free, schema-validated — D10)

| Tool | Kind | Shape | Gate |
|---|---|---|---|
| `bridge.read` | read | `{network?, conv?, since_seq?, limit}` → networks, conversations, recent foreign messages **inside a labelled untrusted block**, edges, adapter facts | none |
| `messages.send` (extended) | write intent | `{to, text}` or `{targets: [{network, conv}], text}` → proposal `message` or `bridge_message` | human accept |
| `bridge.propose_edge` | write intent | `{from, to, why}` → proposal `bridge_edge` | human accept (§8) |
| `devices.read` · `transport.read` | read | DEV §3: device records with raw facts ≤ 4 KB each; interface facts from the `rnstatus` cache | none |
| `device.identify` | consent | `{id}` → the remembered result, or a `device` proposal whose single action `identify` **is** the per-device consent | human accept |
| `device.propose` | write intent | `{id?, actions: Action[], why}` → proposal `kind: device`; 111d validates, applies, verifies, auto-reverts (§12) | human accept |
| `service.restart` | write intent | `{service_id, why}` → proposal `kind: device` with the action `service.restart` | human accept |

- **One global rule (D26 #2; integration B1, bridge M4).** The agent's bearer is refused on **every write**: an explicit gate in `require_session`
  (`N/dashboard/api_v1/auth.rs:974`) refuses a bearer on any non-GET, with **no allowlist**. Today `refuse_bearer` (`messages.rs:69`) is called route
  by route, and `POST /services/{id}/restart` (`services.rs:79`) and `POST /devices/{id}/identify` (`devices.rs:153`) never call it, so the bearer
  reaches a restart and a "consented" probe that carries its own consent flag. The gate closes both and every §10 write; G1 asserts `401` on each. The
  bearer reads every GET, so a route returning credentials is a POST (`/transport/join`).
- **Count.** The tool set grows from 10 to 17 (seven new): bump `A/tests/daemon_tools.rs:306` and `A/tests/chat_loop.rs:308`; schemas
  `A/src/tools.rs:50`, dispatch `:178`.
- **Untrusted foreign text.** Names, ids, bodies and adapter strings reach the model only inside an untrusted block labelled "UNTRUSTED: received from
  other networks", scrubbed like `messages.read` (`clean_text`, 4096 characters, `A/src/tools.rs:539`, `:349`), not `nearby.read`'s 256-character
  `clean_line` (`:529`); the store has already stripped Cc and Cf (§3). **Foreign text never reaches a send or an edge without the human reading it
  first**: the agent can put it into a send only through a proposal whose text renders preformatted (§10).

## 14. Identity and attestation (Layer 2 amendment A10; D26 #3)

A10 (`2-identity/README.md` §13.1) reserved `transport_id` entries for bridge identities and hash binding for 64 B Reticulum keys, to be assigned
"when a bridge lands". The 202 B attestation (`ID/attest.rs:150-170`, §10) has `transport_id` 1 B, `transport_key` 32 B, `transport_sig` 64 B, today
only `0x01` nostr. **Wave 0 lands the bytes and the verifier plug; the gossip kind that would carry attestations between nodes, the `attested-111`
label, "already bridged by node X" and "Open the sealed thread" are the second release (§17).** Requested **(draft proposal)**:

| `transport_id` | `transport_key` | `transport_sig` | Binding |
|---|---|---|---|
| `0x02` meshtastic | the board's X25519 public key (32 B); the node number is learned from NodeInfo, not carried | a 2.7.x board cannot sign (X25519 only, DMs unsigned, R01 §3.18): **64 zero bytes**; a 2.8 XEdDSA-signed broadcast over `"111/attest-transport/v1" ‖ node_pubkey` fills it later | `node_sig` only: **one-sided** |
| `0x03` reticulum | SHA-256 of the 64 B public key (X25519 ‖ Ed25519), as A10 promised | Ed25519 by the identity's signing key over the same input | both signatures |

- **The plug.** `TransportVerifier` (`ID/attest.rs:298`) gets its first implementor in `node111`: for `0x03`, SHA-256 of the supplied 64 B public key
  against `transport_key`, then Ed25519; for `0x02`, an explicit rule that an all-zero `transport_sig` is accepted as one-sided and marked so. Vectors
  are frozen in `id111`; G3 exercises the `0x03` hash binding with the helper's `ready.pubkey` (§6). A one-sided record never yields `attested-111`,
  even later: anyone can write a record naming a victim's board key.
- **No Reticulum arithmetic in our code (carriers M4).** The LXMF destination hash is reported by the helper (`ready.dest`, §6) and **carried and
  compared, never derived**: deriving it from the public key would reproduce Reticulum's addressing from documents, the pattern §12 rejects for the
  RNode probe. How the hash rides in the signed record is pinned with the gossip kind (§17). In R-B `111d` does not read the helper's identity file.
- **One bridge identity per foreign network:** Meshtastic, the board's node; Reticulum, the helper's delivery identity (one `LXMRouter`, one address);
  app-peer accounts have no A10 entry in v1. Link keys stay random, never seed-derived.

## 15. Security, privacy and legal (threat model; hard rules)

1. **Foreign text is untrusted data.** Names and bodies come from strangers: passed through the §3 ingest function (Cc and Cf stripped, NFC, capped),
   rendered as text only (proposal bodies included, §10), handed to the agent inside a labelled untrusted block, never parsed for instructions. On a
   `channel-key` network anyone can use any name; the `auth` label says so. The only body syntax 111 interprets is the relay marker (§8).
2. **The hub holds the plaintext of bridged networks.** App peers terminate the app's end-to-end encryption on the hub (§7). **Residuals, named so
   "remove files" covers them:** the Delta Chat core keeps every contact's decrypted messages in its account store under `bridge/deltachat/` whether
   or not the contact is bridged; the LXMF helper's router keeps state that can hold queued outbound text in `bridge/lxmf/store` (0700, owner
   `node111`). 111 cannot encrypt either; a stolen disk reveals them.
3. **Consent is structural and live.** Edges: a human on a paired or loopback session, DMs unrepresentable, the exact warning recorded, consent facts
   re-read per copy and the edge paused when they change (§8). Per-contact bridging for app peers; HAM confirmation per account; a per-channel uplink
   switch. The agent can propose none of these switches and confirm none.
4. **What leaks where.** MQTT: our packets are published whenever the channel uplinks, PKI DMs as opaque blobs once any channel uplinks (R01 §3.12).
   LoRa public channels: the default key is public. Announces publish an LXMF address and name to every Reticulum peer; a propagation node (opt-in)
   stores others' messages. Public chatmail relays and SimpleX presets are off by default.
5. **Amateur-licensed boards.** `User.is_licensed` is read live; readable sends and relays through that account stay off until a human confirms (and
   per edge); sealed frames never; PKI is off on licensed boards (DEV §0.6; R01 §3.13). Region `UNSET` means the board transmits nothing.
   Third-party-traffic rules are the licensee's fact.
6. **Agent.** Reads freely over the loopback bearer; **writes nothing**: every write, restarts and identify included, is a proposal a human accepts,
   and the bearer is refused on every non-GET (§13). Proposals name conversations by `fc-` id. Foreign text never reaches a send or an edge without
   the human reading it first.
7. **Termux residual (D16 a).** Every app on a phone can reach loopback ports. The Meshtastic downlink has no auth (L5 §14) and the allowlist (§5)
   does not defend against a second local client. The helper listens on no port.
8. **Reticulum posture.** `share_instance = yes` exposes the shared-instance socket to local processes (an abstract AF_UNIX name per network
   namespace, R02 §3.4); `rpc_key` protects the RPC only, in a 0600 file (§11). The Backbone listener exists only while LAN sharing is on. **With
   `enable_transport = yes` and a LAN Backbone without IFAC the hub routes other people's Reticulum traffic, over its own RNode too;** the Settings
   copy says so and IFAC is the limit.
9. **The helper is a trust boundary and inputs are bounded.** Its input is stranger text from the air; `111d` validates every line (size, schema,
   field caps), interprets no field and counts a malformed line as noise. Inbound queues, bodies and `foreign_seen` are bounded; foreign storage is
   evicted by a shaping quota, never refused; a stalled adapter cannot stall the bridge core.
10. **No telemetry.** The helper opens no sockets beyond rnsd's local shared instance; the pip closure downloads at install time only; Madmail, Delta
    Chat and SimpleX default to the self-hosted hub server. Each component is egress-logged in its gate (an AF_PACKET logger plus `strace -f -e
    trace=connect`, R07 §5), and the gate fails on any packet or `connect()` off the declared path.

**Licensing boundary (to land in `LICENSING.md` §2/§3/§5/§7 and `1-bootstrap/TASKS.md` sign-off rows):**

| Item | Status |
|---|---|
| shipped `111d` binary | effectively **GPL-3.0-only** once the `meshtastic` crate and prost types link in (D25 F2); our files stay "or later". The protobufs `LICENSE` has no version notice, so a legal read decides whether "or later" survives (GPLv3 §14; R06 G9). Rows: the crate and its 49-crate closure (MPL-2.0 `serialport` unless dropped), prost, bytes, prost-derive, the one protobuf snapshot (commit hash and checksum beside the notices) |
| `lxmf-sidecar` and Reticulum | **pending the owner's contact with upstream.** A separate process in its own venv, never combined with `111d`, importing the user-installed `rns` and `lxmf`; Reticulum's authorship credited in its header, README, `THIRD-PARTY-NOTICES.md` and Settings. Its own licence, and how it sits with the Reticulum License (non-OSI; no-harm and no-AI-training clauses, `LICENSING.md` §8), follow that contact and a legal read (R02 §4 C). `LICENSING.md` §1, §7, §8 and the evidence line at `:30` name the helper as the one program of ours that imports Reticulum (D17 #12 amended, D26 #4); "no crate depends on Reticulum" stays true |
| every third-party program a component needs | downloaded hash-locked from its public source at install or enable time, **never in a 111 payload** (D26 #5). Before any download or enable 111 shows a notice (program, licence, where to read it) and records the user's acknowledgement: the user installs, 111 and its author are not the distributor. The Layer 1 rns venv gets the same one-line notice |
| Madmail · SimpleX · Briar headless | AGPL-3.0 (Madmail: repo metadata only; **its licence file is read before its pin**, its AGPL §13 source offer named) · AGPL-3.0 · GPL-3.0-or-later per LICENSE.txt: separate processes; source offers name the pinned tags |
| `deltachat-rpc-server` · test tools | MPL-2.0, separate process · `meshtasticd` (GPL-3.0) and Meshtasticator (CC BY 4.0) are test fixtures, never shipped; Sideband (CC BY-NC-SA) is never shipped; Columba is linked, not shipped |
| notices | `gen-notices.py --check` hashes the lxmf lock and every component pin and lists a row per lxmf-closure package |

## 16. Verification (lock gates; netns, no hardware)

**Gate homes (integration M6).** New `6-transport/verify/{bridge,lxmf,spine,devices}.sh` (G1, G3, G4, G5), each skipped without its prerequisites
(unprivileged netns, `venvs/rns`) unless its `REQUIRE_*` is set for a lock run, as `REQUIRE_G2` does; `1-bootstrap/scripts/check.sh` gains a
`6-transport` stage.

**G1, bridge (`verify/bridge.sh` plus `cargo test`).** One `111d` (two on a veth for the cross-node loop cases) with three `StubAdapter`s of differing
caps. Asserts: foreign rows never reach `/messages/threads` · **`401` for the agent bearer on every Layer 6 write, on `POST /devices/{id}/identify`
and on `POST /services/{id}/restart` (a `node111` test)** · fan-out: `refuse` answers `409` and queues nothing, `split` makes per-part rows and cancel
drops unsent parts only, adapter-state failures queue with a reason · relay: a proposal creates nothing until accepted and the 409 round-trip carries
text and hash, history before enable is not relayed, `relayed_seq` holds after a shed, a relayed copy is never relayed, **split both ways on two nodes
with reverse edges does not loop**, **a board named `]` or `W1AW/7` does not loop**, own-echo from any account, DM and `src = dst` refused,
`honour_ok_to_mqtt` on and off, a change in uplink or licensed state pauses the edge · the ingest function strips U+202E and zero-width characters ·
enabling a component without the recorded acknowledgement answers `409` · a `pending` row neither blocks later rows nor is re-sent after a crash ·
user rows go ahead of relay copies · the **1,000-row burst** keeps 111 delivery latency under its bound · airtime refusals · prune order, the
`foreign` quota and bounded `foreign_seen` · a V3 store is refused by a V2 schema (a mesh111 unit test with `user_version = 3`) and V2 opens at V3.

**G2, the Meshtastic client (a `node111` `cargo test` with a test-only prost stub).** The L5 harness stub (`4-interface/verify/stub-meshmud.py`)
speaks JSON, not protobuf, so it cannot serve; the client counts undecodable frames as noise, which keeps that harness passing with the client on.
Asserts: one `want_config_id` per connection, never 1, 69420, 69421 · replays ingested without duplicates · text before `config_complete_id` buffered
and mapped · nonce-0 heartbeat on the test tick · the allowlist drops and counts everything else · **a dropped second `want_config_id` does not wedge
the crate** · spacing ≥ 2.1 s on a fake monotonic clock · `free = 0` pauses · every `Routing.Error` of §5 maps as specified · `want_ack = true` and
the reliable-record path · `key_unknown` resumes on a NodeInfo with a key · region `UNSET` · `via_mqtt` tag · live `uplink_enabled` and `is_licensed`
changes · **a sentinel PSK appears in no log, row, API body, SSE frame or agent output** · a prost encode fixes the shapes **text 232 / 220,
`PRIVATE_APP` 231 / 219** against the firmware limits 239 / 227, which are *src* and settle on hardware (R06 G7; L5 §8 says 233 / 221) · the shim
drops `ToRadio.disconnect` · Nearby parity per field on one committed `FromRadio` fixture. Multi-node behaviour (Meshtasticator, `meshtasticd -s`)
waits for a native firmware build in CI; **neither binary has been obtained, so it is not yet evidence**.

**G3, the LXMF helper (`verify/lxmf.sh`; supersedes the old harness rule).** D26 #4 supersedes the rule that no harness may import RNS or LXMF (R02
§3.18, `LICENSING.md:30`): the helper's tests and G3 import them inside the helper's venv; `111d` and its Rust tests still import nothing of
Reticulum. G3 runs **two netns, each with its own real `rnsd` (A a Backbone server, B a TCPClient) and one instance of our helper; the far end is the
second helper** (R07's shape: `unshare -Urnm`, veth). Asserts: the helper attaches as a client and never becomes the instance; a mis-ordered start is
detected as a partition and recovered · both directions, hash dedupe, the custom-field round trip, `signed` true once the source announced and false
with `why_unsigned` before · announce before reply (`peer_unannounced`, then success on `heard`) · monotone states, a late `failed` after `delivered`
ignored · delivery in seconds with no stamp cost · an rnsd restart: one delivery, no regression · an AF_PACKET log of **zero** off-link packets ·
enable (acknowledged) → disable → "remove files" leaves nothing installed.

**G4, the spine (`verify/spine.sh` plus unit tests).** Golden renders per posture and stanza set · **modes: `rns/config` 0600, `rns/` and the helper's
client dir 0700** · real `rnsd` on the rendered dir → `rnstatus` facts · live attach, detach, reload · the posture restart path (the helper stops,
rnsd restarts, the helper returns) and a double crash that must not partition · an RNode stanza that cannot open shows down while rnsd stays up · the
second-AutoInterface case **recorded, not assumed** · phone relay in two netns, "Clients: 1" · the QR-join flow between two `111d`s · migration from
`devices.json` and `rns/config`, the TOML winning a divergence with its log line · a v2 file left alone by v1 (a unit test of
`existing_schema_version`, `N/transport/config.rs:45`).

**G5, the device core (`verify/devices.sh`).** uevent fixtures · `PortTable` collisions among rnsd, 111d, meshmud and an identify probe answer `409
port_busy`, and a foreign loopback holder is caught by the pre-bind check · the identify sequence against a **fake `rnodeconf`**, a stub meshmud and a
**MeshCore pty stub modelled on the MIT `kiss_modem_protocol.md`** (never `RNodeInterface.py`); `unknown` only after three `NoMatch`; a missing tool
is `Error`; consent required; the 30 s rate limit · the applier's four actions with injected probe failures, a crash mid-apply, `unsupported_action`,
and the two one-shot actions.

**G6, reserved.** R-B hosts no panel for an adopted program, so `panel-lint.sh` has nothing to lint. When one returns (§17) it gains `--panel <id>`
and this gate (lint the server-side upstream session; tag *design* until built).

**G7, the full gate and the zero-weight gate.** `1-bootstrap/scripts/check.sh` with `REQUIRE_G2=1`, `5-messaging/scripts/check.sh` (V3),
`3-intelligence/scripts/check.sh` (17 tools), `pnpm check` (lazy chunk ≤ 64 KB), the presence-sidecar tests until it retires. **Zero weight is
runtime weight:** with every component disabled, processes and listening ports equal the Layer 5 baseline and **nothing is installed** (no
`components/`, no `venvs/lxmf`); cached payload size, idle RSS and the binary-size delta (both musl targets) are recorded, not asserted. **G8:** the
fable re-check on the diff, then the lock entry.

**External gates before the first signed release (hardware; none run).** Serial open without an unwanted reset under meshmud, `rnodeconf` and the
MeshCore probe · panel plus bridge sends against the shared 2 s window · implicit versus real ack on two boards · a DM with and without a known key ·
the `ok_to_mqtt` bit position · whether the firmware returns our own rebroadcast · a licensed board · an uplinked channel · the 239 / 227 B limits ·
2.8.1 numbering and signing · Termux: `meshmud` linux/arm64 and an rnsd `TCPClientInterface` to the hub without multicast · Columba ↔ hub rnsd over
TCP · Delta Chat phones against the Madmail relay (attended; only if the spike passed) · for the second release, the two-board KISS modem bench and
the MeshCore data-type PR.

## 17. Deferred / second release

- **Sealed 111 carriage over foreign radios (D25 F6):** the `meshtastic-client` and `rns-bridge` links and the six mesh111 amendments R00 §3 lists
  (opaque per-peer connections, a shim codec, re-planning unsent fragments, a cost-aware route, `remove_link`, a non-blocking `send`). **The 111 KISS
  modem link, its CI image and the MeshCore data-type PR** (dev range `FF00–FFFF`; the registry grants an ID only after a working demo, R05 §3.4).
- **Attestation labels (D26 #3):** the gossip kind (an L5 frame kind, a store table, a publisher), `attested-111`, "already bridged by node X", "Open
  the sealed thread", and how the helper-reported LXMF destination hash rides in the signed record.
- **Hosted panels and the child-panel proxy:** the generic proxy L5 §13 reserved (WebSocket, SSE, large bodies; `manifest.rs:400` refuses `child`
  today) and any `ServiceId` it needs (a panel id must be one, `manifest.rs:10-11`). **MeshChatX is a possible optional hosted panel later** (0BSD AND
  MIT, headless; R02, R07 ran it; its upstream ends 2027-01-01 and its closure is heavy), never the carrier. G6 un-reserves with it.
- **Layer 7:** runtime join-in-sequence and fall-back (README §6), presence, gateways. **v1.1:** hub-member routing of foreign DMs. Termux as an LXMF
  host is a bench item.
- **Not built:** a Meshtastic key request, `meshtastic.set`, BLE pairing, `device.flash`, the USB-to-TCP and MulticastLock helper APK, analog radios,
  MeshCore companion identify, a distribution-group hub or Reticulum Relay Chat rooms, relays out of Reticulum, two-bridge coordination, a courtesy
  notice, an "All networks" feed, request semantics for strangers' DMs, Matrix or XMPP gateways, MeshMonitor, Briar offline.
- **Open owner items, with the default taken:** public chatmail or SimpleX servers (off) · the Columba APK (link only) · LXMF display name and
  announce interval (§6) · the helper's licence (pending upstream contact) · which programs run as natives (none decided).

## 18. Cross-layer seams & amendments

- **Layer 5 (locked v1.2), additive:** schema V3, `Class::Foreign` with its shaping quota (§10.4), typed batched `Fabric::foreign_*` and
  `FabricEvent::Foreign` (§3) · §14's "111 never transmits", "read-only" and "no Meshtastic protobuf code enters our Rust binary" restated (§5, §15),
  with the doc comments in `N/meshtastic/mod.rs:5-7`, `shim.rs:9-10` and `M/link/mod.rs:233` updated (the Link seam waits for the second release) ·
  "Message them" is no longer withheld (L5 §14) · the presence reader retires after parity · `interfaces.toml` is the only reader of the Meshtastic
  binding (§11) · INTRO over LoRa stays deferred (`M/fabric.rs:2475`).
- **Layer 2:** A10 bytes `0x02`, `0x03` and the `TransportVerifier` implementor with frozen vectors (§14) · one domain-tagged derivation on the
  identity handle for `111/bridge-conv/v1` (the A9 pattern); Cat-1 vectors untouched.
- **Layer 1:** new children, dirs and ports (§2) · `ChildSpec.stdio` with a supervisor-owned reader (`N/supervise.rs:55,449`) · **child ordering is
  bridge-core orchestration over `stop_and_wait`/`restart` and fresh spawns, with the post-spawn partition check; not a supervisor feature** (task
  cut, G4) · `111 install` pre-downloads declared payloads into `cache/packages/<id>/` and prints the notice line, recorded in the install receipt
  beside the artifact hashes, and the rns venv gets the same line (D26 #1, #5) · `venvs/lxmf` and its lock in the signed manifest (hashes only, no
  Reticulum bytes) · renderer modes (§11) · `CAP_NET_BIND_SERVICE` for Madmail if it needs standard ports, `AF_NETLINK` kept if the unit restricts
  address families · `gen-notices.py` and `LICENSING.md` rows (§15) · the `rns` pin resolves from PyPI at release time (`1-bootstrap/README.md:45`
  still says `rns==1.5.6`) and the helper's lock agrees with it (asserted by `gen-notices.py --check` or the release checklist).
- **Layer 4:** routes, SSE, enums and pins (§10) · the **global bearer write gate** in `require_session` (`N/dashboard/api_v1/auth.rs:974`), which
  also closes `restart` and `identify` · preformatted proposal bodies · `pnpm gen:types`, `gen-sse.mjs --check` · `/settings/l6` · agent prompts gain
  `bridge.setup` and `transport.fix` (`UI/lib/prompts.ts:124`). **Layer 3:** seven tools and the count pins (§13).
- **D17 #12 and `LICENSING.md`:** amended by D26 #4 (§15). The rule that no test harness imports RNS or LXMF is superseded for the helper's tests and
  G3 only.
- **Layer 0 (KB), new `candidate` entries:** `meshtastic-rust` · `madmail` · `deltachat-rpc-server` · `simplex-chat` · `rnode-firmware` (cache-seeding
  recipe) · `ble-reticulum` · `leviculum` · `meshtasticator` · concept `preferred-networks`; updates to `meshcore` (allocation rule), a Meshtastic 2.8
  region table from `config.proto`, and `meshchatx` (no longer the carrier). **Stale:** `projects/reticulum.md:52` and `lxmf.md:44` (PyPI has RNS
  1.5.7, LXMF 1.2.1) · `meshtastic-python.md:36,60` and `meshmud.md` (in-binary protobufs no longer a blocker, D22) · `briar-bramble.md:9`
  (`GPL-3.0-or-later`).
- **Layer 7 inherits:** runtime join and fall-back · presence (NodeInfo and Reticulum announces already feed Nearby) · hub-to-hub replication ·
  attestation gossip and labels · the hub-learned `custody_seq_floor` (L5 §7.3) wherever a join order needs it.

## 19. Plan corrections recorded (from OB §4 and D26)

- D20 hand-off "MeshCore data-type range registered before Layer 6 locks" → dev range `FF00–FFFF` until the two-board bench, then the PR (R05 §3.4).
  `DEV` §8 "parse rnsd config with `--exampleconfig`" → start rnsd on a scratch dir and read `rnstatus` (R05 §3.10).
- L5 §14 "111 never transmits" and "no Meshtastic protobuf code enters our Rust binary" → restated per D22, D23, D25 (§5, §15). The reader's missing
  heartbeat (R01 §3.9) is built (`meshtastic_presence.py:35`). L5 §8 budget 233 / 221 B → 231 / 219 B (`PRIVATE_APP`), text 232 / 220 B (shapes by
  prost encode in G2, firmware limits on hardware).
- `DEV` §3 "own probes" → adopted `rnodeconf --info` (R06 G6); `DEV` §3 `service.restart` "control, rate-limited" → a proposal like every write (D26
  #2). The `DEV` §3 closed action vocabulary grows by two one-shot actions, `identify {device_id}` and `service.restart {service_id}`, carried in
  `kind: device` proposals (one applier loop, fewer pins); `identify` renders on a device card and `service.restart` on a service card.
- D17 #12 → amended by D26 #4: Reticulum stays a separate service never incorporated into `111d`; the LXMF helper is the one named program of ours
  that imports it, as a separate process. OB F3 (MeshChatX closure, bundled CPython, carrier trait, catch-up rules) → withdrawn; R07 stays as evidence
  for the Reticulum side only.

## Changelog v0.1 → v0.2

- **Header, §0, §1:** status, reading list, waves (proxy and panel out), hard rules and owner quotes (D26 #1–#7).
- **§2, §2.1:** MeshChatX, CPython and the proxy out; ports and pre-bind check; `venvs/lxmf`, `cache/packages/<id>/`; child ordering as bridge-core
  orchestration; seam rewritten, `delivery` dropped (carriers B1, M3, M5, M8, minor 12; integration B3, M1, M8).
- **§3, §4:** `part_k/n`, `relayed_seq`, `foreign_seen`, denormalised kinds, batched ingest, ingest function, dedupe window, `signed`, shaping quota;
  tick retries, `ForeignMessageState`, post-air NAKs, pending rows, queue order, overflow counter (bridge M1, M2, M6, M7, M9, minors 1, 4, 8, 10, 11;
  integration M2–M4).
- **§5:** `want_ack`, feature trim, one snapshot, pre-`config_complete` buffer, live consent, parity per field (bridge M2, M3, minor 12; carriers
  minors 1–5, 13; integration M5). **§6:** rewritten around `lxmf-sidecar` (D26 #4, #5; carriers M2, M6 moot). **§7:** spike first, one account
  warning, supervisor reader, Termux refusal (D26 #6; carriers M7, M8, minors 10, 11; bridge minor 13).
- **§8, §9, §10:** marker alphabet, parts and parser, live pause, accept-time 409, `fc-` ids, one row per part; routes, enums, pins, preformatted
  bodies, unread-only attention, per-conversation SSE, no `ServiceId` growth (bridge B1, M3, M6, M8, minors 2–4, 6, 9, 15; integration M2, minors).
- **§11, §12, §13, §14:** ids once, region override, TOML wins, modes 0600/0700, `rnstatus` cache; hotplug notes, `identify` and `service.restart` as
  `device` actions; global bearer gate and scrub shape; bytes and verifier only, no derivation (integration B1, B2, M7; carriers M1, M4; bridge M4,
  M5, M9; D26 #2, #3).
- **§15–§19:** stores, live consent, helper licensing rows, notices; G1–G7 reshaped, G6 reserved, gate homes; MeshChatX and labels to §17; Layer 0/1
  rows (carriers M5, minors 9, 14, 15; integration M5, M6; D26 #3–#5).

## Changelog v0.2 → v0.3

- **1** §6 (4), (6): a post-air LXMF `FAILED` keeps `sent`, `Retry::Later` on `heard`; `OUTBOUND` + `FAILED` is `queued`/`peer_unannounced`; only
  `REJECTED` and `CANCELLED` are `failed`. **2** §6, §18 Layer 1: the helper's lock agrees with `rns-lock.txt`. **3** §2.1 rule 3, §18 Layer 1: the
  notice is recorded in the install receipt. **4** §18 Layer 4: agent prompts restored.
- **5** §12, §19: the two one-shot actions in `kind: device` recorded; restarts render a service card. **6** §2: port 11232 only. **7** citations
  re-pinned (`supervise.rs`, mesh111 `fabric.rs`, `API:920`, presence reader, `1-bootstrap/README.md:45`) and the header note. **8** §6 (1):
  require-shared-instance tagged *(unverified)*, the "not measured" cite dropped. **9** §15: `LICENSING.md` §8. **10** §6: why piped stdio. **11** §6,
  §14: `ready.pubkey`. **12** §6: Sideband and Columba are the native UIs. **13** §2.1: updater manifest refresh (`LICENSING.md` §9.3).

