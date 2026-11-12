export const meta = {
  name: 'layer-7-wave-1',
  description: 'Layer 7 build Wave 1 per 7-relay/TASKS.md: D2-D10 the daemon side (presence, gateway row, netlink, TLS, pairing, origin, DNS-SD, roles, relay status), U1-U3 the UI, A1 agent tools, K1-K2 spec text; three stages for the landing order; two slots; the lead reviews and commits each',
  phases: [
    { title: 'Stage A', detail: 'D5 · D4 · D9 · D3 · D7 · U1 · U3 · K1 · K2 (disjoint files); each code row reviewed read-only as it returns' },
    { title: 'Stage B', detail: 'D6 (after D5) · D8 (after D4) · D2 (after D9) · A1 · U2' },
    { title: 'Stage C', detail: 'D10 (last: assembles D1-D9)' },
  ],
}

const a = args || {}
// args.root: a git worktree of the repo to build in (the lead applies its diff to the main tree), so Rust rows can run while another agent
// edits mesh111 in the main tree. Default: the main tree.
const ROOT = typeof a.root === 'string' ? a.root : '/home/user/os'
const SP = '/tmp/claude-0/-home-user-os/f8991dd8-132d-5f0d-807d-446e0e7eb394/scratchpad'
const ONLY = Array.isArray(a.only) ? a.only : null

// What Wave 0 landed (the lead fills this from the committed tree before the launch; the rows build on these names).
const WAVE0_FACTS = `
WAVE 0 LANDED (read the files, not this summary, before you rely on a name):
- mesh111 store (M1, 57af4bc): schema v4; \`Fwd::{Member, FromMember(id), FromLeaf(id), FromCarrier(id)}\`; \`custody_put\` unchanged (a member row),
  \`custody_put_fwd(dest, bytes, Fwd)\`; \`Class::Transit\` (\`quota(Transit)\` always Some; \`transit_share()\` = quota / 8); the forwarder API
  \`forward_pending(after, limit)\`, \`forward_item(seq)\`, \`forward_by_ref(pkt_ref)\`, \`forward_commit(seq)\`, \`forward_refuse(seq)\`, \`forward_stats()\`,
  \`forward_origin_bytes(origin)\`; node111's \`json::quota_classes()\` now yields \`transit\` too (GET/PATCH /fabric/settings carry it).
- mesh111 envelope (M2, ef84094): \`Ext.dest_hub: Option<[u8; 8]>\`, \`TLV_DEST_HUB\`, \`DEST_HUB_LEN\`, \`DEST_HUB_TLV_LEN\` = 10, \`DEST_HUB_ALONE_LEN\` = 11,
  \`EnvelopeError::{DestHubNotAllowed(Kind), DestHubLength(usize)}\`; vectors \`5-messaging/test-vectors/layer7/dest-hub.json\`.
- mesh111 outbox (M3, 7746eef): \`Reason::Forwarding\`, code \`forwarding\`, sentence "Your hub holds it and passes it to their hub."
- mesh111 link (M6, 22de995): \`tcp_lan::StreamWrap { outbound(addr, expected: Option<NodeId>, TcpStream) -> BoxFuture<io::Result<BoxedStream>>;
  inbound(remote, TcpStream) -> BoxFuture<io::Result<(BoxedStream, Option<NodeId>)>> }\`, \`TcpLanConfig.wrap: Option<Arc<dyn StreamWrap>>\` (None =
  byte-identical); the inbound pair named by the wrap is checked against the first announce inside the handshake (S7); \`LinkEvent::Connected.expected\`
  stays the dial's (None for a wrapped inbound).
- node111 carrier (P1): \`N/carrier/ntfy.rs\`: \`NtfyConfig { server, token: Option<Zeroizing<String>>, keepalive, connect_timeout, request_timeout, trace }\`,
  \`NtfyCarrier::new(cfg)\` (checks the §6.3 server rule: https://, or http:// for a private IP literal or localhost), \`impl Carrier\` (\`flush\`, \`drain\`,
  \`hold\`), the cursor API \`cursor(pair)\`, \`restore_cursor(pair, id)\`, \`commit_drain(pair)\`, the prototype helpers \`drain_into(pair, &dyn HandOff)\` and
  \`flush_5a(pair, &dyn HandOff, max)\`; topics \`topic_key(key, enc_pk_a, enc_pk_b) -> Option<Zeroizing<[u8; 32]>>\`, \`outbound_topic(pair)\`,
  \`inbound_topic(pair)\`; \`N/carrier/mod.rs\` keeps node111-local \`CarrierFrame\`, \`ForwardDone\`, \`HandOff\`, \`Pair { own, peer, topic_key }\` until D11.
  The stub server \`7-relay/verify/stub-ntfy.py\` and \`carrier.sh --smoke\` exist (no real server reachable from this box).
- the contract (C1): \`4-interface/api/openapi.yaml\` "Layer 7" block is the binding form of every route and shape (grep 'Layer 7'); read it before
  building a route: \`RelayStatus\` (required: schema, gateway, carrier, paired_hubs, forward, presence, discovery, origin, updates, metered; secrets as
  \`{set: bool}\`), \`RelaySettingsPatch\` (carrier keys FLAT: \`carrier.{kind, server, mode, token, topic_secret}\`, secrets write-only strings, null clears;
  \`accept_warning\`), \`RelayGateway\` (enabled, state, carrier, endpoint, mode, checked_ago_s, last_flush_ago_s, pending, metered_cap_reached,
  termux_poll_min?), \`PairedHub\` (node_id, label, role, via, tls rpk|plain|none, lan, reachable, pending, reposts), \`PairOfferRequest\`, \`PairOffer\`
  (schema, bundle: Card, code, link_spki), \`PairRequest\` (bundle, role, via, lan = false, accept_warning?), \`RelayAccepted {schema, started}\`,
  \`RegistryRow\`, \`UpdateOrigin\` (kind hub|internet|directory|url, url, last_answer {ok, ago_s, detail}|null, newest_seen), \`GatewayDetail\`,
  \`GatewayOffer {carrier, mode}\`, \`NearbyEvidence {transport ∈ tcp-lan|hub|carrier, text}\`; \`Problem.code\` + no_such_transport, licensed_board,
  metered_cap, lan_off, pair_role_mismatch; \`ConsentRequired.action\` + carrier_enable, pair_plain, gateway_bit, origin_serve; SSE \`relay\` (a full
  RelayStatus) and \`registry\` ({rows}); the \`gateway\` service row's codes gateway_none|carrier_ok|carrier_pending|gateway_off. UI side: the lazy
  adapter \`UI/lib/api/relay.ts\` (\`RelayAdapter\`, \`createHttpRelay\`, \`relayConsentOf\`) reached through \`getRelay()\` in \`UI/lib/api/index.ts\`; the
  dev mock \`UI/lib/api/mock/relay.ts\` (\`adapter.mock.relay\`, \`relayControl\`); store slices \`relay\` and \`registry\` with \`refreshRelay\`/\`refreshRegistry\`;
  \`UI/lib/api/nearby.ts\` gains \`isOneOneOneSource\`, \`nodeEvidence\`, \`heardAgoNow\`, \`ageLabel\`; \`proposal-body.tsx\` renders a \`relay\` body as a card.
  node111 mirrors: \`proposals::KINDS\` is 9 with \`relay\`; \`nearby::EVIDENCE_TRANSPORTS\`, \`kind_for("111")\`; \`BridgeNearby::push\` never admits a \`111\` source.
  Known gaps for Wave 1: \`bridge.ts\` \`CONSENT_ACTIONS\` (its own closed list) drops the four new actions (U2); \`mock/fixtures.ts\` gateway row is still the
  old shape (U3); the two \`quotas\` descriptions in openapi.yaml do not name \`transit\` (D10).
  The contract-walk test \`N/tests/dashboard_auth.rs\` (\`the_agent_bearer_is_refused_on_every_write_of_the_contract\`) lists the seven Layer 7 write
  operationIds (patchRelaySettings, offerRelayPair, pairRelayHub, unpairRelayHub, checkRelayCarrier, flushRelayCarrier, browseRegistry; and
  runNearbyHops, which D2 routes) in its \`UNROUTED\` allowlist because they have no route yet: the row that routes one REMOVES it from \`UNROUTED\` (the walk then expects 401 for the agent
  bearer, no 401 for trusted loopback and a paired session, on each) and keeps \`cargo test -p node111 --test dashboard_auth\` green.
- node111 manifest (L1): \`ChannelManifest::parse_verified\` (same signature) walks the keychain from the compiled key (T2), verifies under the highest
  generation reached, refuses any channel but \`STABLE_CHANNEL\` (T1), cuts the keychain to the generations walked and exposes \`trusted_generation()\`;
  caps \`MANIFEST_MAX_BYTES\`, \`SIGNATURE_MAX_BYTES\`, \`KEYCHAIN_MAX\`; fixtures \`tests/fixtures/l7-L1/\`.
- node111 updater (L2): \`fetch::Origin::{HomeHub {base}, Internet {base}, Dir {path}, Url {base}}\`, \`Origin::home_hub(host) -> Option<Origin>\` (IP literal or
  plain DNS name → \`http://<host>:11180/origin/v1/\`), \`Origin::{kind(), label(), is_internet()}\`; \`UpdaterConfig.check_internet\` (default true; static
  until D10/L4 wire it), \`UpdaterConfig::{internet_origins(), visible_order(home_hub: Option<Origin>)}\`; \`fetch::check_origins(cfg, layout, &[Origin],
  LockWait::{Wait, NoWait}) -> FetchOutcome\` (\`Held\` on a NoWait take of a held lock, \`NoOrigin\` on an empty list, \`AlreadyStaged\` when the run it
  waited for staged the same release); \`check_once\` = the internet bases under Wait; \`stage::StagingLock::{acquire(state_dir, wait) -> LockTake,
  waited()}\` (\`state/staging.lock\`, flock, O_CLOEXEC | O_NOFOLLOW, 0600); the blob rule (a non-internet origin fetches \`blob/<sha256>\`, never
  \`payload.url\`); T3 (\`min-upgrade-from\` above the installed release ⇒ Refused before any payload); the per-origin record
  \`state/update-origins.json\` (\`fetch::origin_records(state_dir) -> Vec<OriginRecord {kind, origin, answer, release, newest_seen, detail, at}>\`;
  internet bases keep \`last-update-check.json\`); \`UpdaterStatus::settle()\`; \`util::http::{check_url(url, Option<HttpException>) -> Result<Scheme,
  UrlRefused>, HttpException::{HomeHubOrigin, UserTypedFrom}, Fetcher::allowing_http(e), Fetcher::get_capped(url, max), is_not_found(&err)}\` (T6:
  https:// always; http:// to a loopback IP literal, or to any host under a typed exception; no redirect on http://, https-only redirects on https://).
- node111 relay settings (D1): \`N/relay/settings.rs\`: \`relay::settings::{load(state_dir) -> Loaded {settings, invalid}, save(state_dir, &s),
  update(state_dir, |s: &mut RelaySettings| -> Result<(), E>) -> Result<(RelaySettings, bool), UpdateError<E>>}\` (no file until the first real change;
  an invalid file is moved aside to \`relay.json.invalid-<n>\` and reads all off); \`RelaySettings\` getters/setters per §6.3 key (\`gateway_enabled\`,
  \`termux_poll_min\`, \`carrier_kind\` (\`CarrierKind::{None, Ntfy}\`), \`ntfy_server\`, \`ntfy_token\`/\`ntfy_topic_secret\` (\`Secret\`, Debug prints only
  "set"; \`ntfy_topic_secret() -> Option<Zeroizing<[u8; 32]>>\`), \`carrier_mode\`/\`carrier_mode_for(termux)\` (\`CarrierMode::{FiveA, FiveB}\`; 5B refused
  on Termux), \`consent_hash(ConsentKind)\`/\`set_consent_hash\` (\`ConsentKind::{CarrierEnable, GatewayBit, OriginServe}\`), \`heartbeat_min(transport)\`
  (read-only: no setter in v1), \`lan_record\`, \`public_gateway_bit\`, \`offload_bit\`, \`origin_serve\`, \`origin_targets\` (the own target first),
  \`check_internet\`, \`metered\`/\`set_metered(Vec<MeteredLink>)\`, \`metered_window\`/\`set_metered_window\` (CLOCK_BOOTTIME slots + boot_id: relay::metered's
  to bucket), \`drain_cursor(topic)\`/\`set_drain_cursor(topic, id)\`); \`view() -> SettingsView\` serialises to the contract's RelayStatus keys this file
  owns (gateway.enabled/termux_poll_min, carrier (secrets as {set}), presence.heartbeat (the three Layer 7 transports, owner layer7; D10 appends the
  rns row), discovery, origin.serve/targets, updates, metered). Not here: quota.transit (the store), the pairs and pair_plain's hash (fabric/settings).
  The file uses deny_unknown_fields on every object: a new key is added here, never written blind.
- node111 update helper (L3): \`update_helper::check_min_upgrade_from(installed: Option<Calver>, &manifest)\` (T3; an unreadable installed release is
  refused too); a \`--from\` staged bundle passes the same chain as an internet one.
- M4's minimal node111 compile fixes already in the tree: \`Counters.forward_refused\`/\`forward_too_large\` (mirrored by RelayStatus.forward, not
  FabricCounters), a \`FabricEvent::Forward { .. }\` arm in \`FabricHandle::publish\`, \`paired_hubs: Vec::new()\` in fabric/settings decode.
- mesh111 fabric (M4, 841d90a): \`FabricSettings.paired_hubs: Vec<PairedHub>\`; \`PairedHub { node_id, role: PairRole, via: PairVia }\` (\`valid()\`: a carrier
  pair is \`Peer\` only); \`PairRole::{Peer, Leaf, Gateway}\` and \`PairVia::{Tcp, Carrier}\` with \`code()\` ("peer"/"leaf"/"gateway", "tcp"/"carrier"),
  \`from_code(&str)\`, \`PairRole::complement()\` (peer↔peer, leaf↔gateway); re-exported at the crate root: \`mesh111::{CarrierFrame, ForwardDone, PairRole,
  PairVia, PairedHub}\`. Handle API (every call refused on a node that holds no custody): \`Fabric::pair(address: Option<SocketAddr>, code, card, role, via)
  -> Result<PairedHub>\` (hub role only; the ENROLL rides a connection already attributed to the far hub when one exists, else dials \`address\`; the far hub is
  pinned \`oob\` and the pair is WHOLE only once the far hub enrolled here too; a paired hub is never the home hub, am. 12), \`unpair(node_id) -> Result<bool>\`
  (the far hub's contact goes back to \`oob\`, its puts then meet REJECT_UNENROLLED), \`set_carrier_max_frame(Option<usize>)\` (in memory; \`None\` = no
  carrier target for a new put), \`forward_take(target, max) -> Result<Vec<CarrierFrame { custody_seq: Option<u64>, bytes }>>\` (acks first, \`custody_seq
  = None\`, consumed; then puts whose turn it is), \`forward_done(custody_seq, ForwardDone::{Taken, Rejected { why: &'static str }, Retry})\`,
  \`carrier_inbound(from, frame: Vec<u8>)\` (size, then open, then the counter window; CUSTODY_PUT and CUSTODY_ACK only), \`forward_repost(target) -> Result<u32>\`.
  \`FabricEvent::Forward { target: NodeId, pending: u32 }\` (coalesced per carrier pair per tick; emitted when a row's turn or an ack reaches the pair, a
  post is re-armed, or the bound is set; never after a \`Retry\`). \`FabricStatus.paired: Vec<PairView { hub, whole, broken, reachable, pending, reposts }>\`
  (the member count skips pairs); \`Counters.forward_refused\`, \`forward_too_large\`. Constants \`MAX_PAIRED_HUBS\` 32, \`FORWARD_CACHE\` 256,
  \`FORWARD_WINDOW\` 32, \`CARRIER_ACK_QUEUE\` 1024, \`FORWARD_PUT_WRAP\` 131, \`pub fn carrier_fits(max_frame: Option<usize>, item_len)\`.
  Readings built in: a tcp pair's outbound link-up sends one MAILBOX_FETCH {0}, answered with one item-less MAILBOX_ITEM (both sides attribute); a tcp
  pair's refusals end with its connection; REJECT_FULL rows retry on the pair's link-up/attribution, any other ack from it or a row inserted for it; the
  \`forwarding\` reason only when the hint names a hub this node knows (hub_flow's stale-hint test unchanged; OWNER QUESTION, board row M4).
- node111 fabric seams (M5, ca4c514): \`N/fabric/settings.rs\`: \`PairLink { node_id, address: Option<SocketAddr>, link_spki: Option<LinkSpki>, lan, warning_hash:
  Option<[u8; 32]>, far_holds_address }\` (\`PairLink::new(id)\`), \`LinkSpki::{from_der, from_text, as_der, raw_key, text}\` (44 B Ed25519 SPKI DER, 71-char card
  base32), \`PairTable = Arc<RwLock<Vec<PairLink>>>\`, \`Persisted.pairs\` + \`Persisted::pair_link(&id)\`, \`pair_link_refusal(&link, &own, refuse_loopback)\`,
  \`pair_dials(own, &Persisted) -> Vec<(NodeId, SocketAddr)>\` (hub role only; tcp pairs; this side dials when \`!far_holds_address || own < far\`:
  \`far_holds_address\` means "this hub's bundle showed the far hub an address", D6 sets it), \`holds_address(&Persisted, addr)\`, \`Recover.pairs\`; the file's
  \`paired_hubs[]\` rows \`{node_id, address, role, via, link_spki, lan, warning_hash, far_holds_address}\` (the key omitted when empty; address/link_spki
  optional; any other bad row makes the file invalid); \`rebuild_home_hub(enrolled, hint, announced, pairs)\`. \`N/fabric/mod.rs\`:
  \`FabricHandle::{set_pair_link(PairLink) -> Result<(), FabricError>, forget_pair_link(&NodeId) -> bool, pair_table() -> PairTable,
  replace_home_hub_address(addr, wait: Duration) -> Result<bool, FabricError>}\` (D6 calls set_pair_link BEFORE Fabric::pair so the pin is in place; the half
  reaches the file with the fabric's record), \`reconcile_pair_dials\` (after every pair change; through \`Fabric::dial_peer\`), the tap \`relay::tap(&ev)\` for
  Message/Contact/Fabric/Forward events (D2/D3/D11 consume it; a no-op while off), \`TcpLanConfig.wrap = relay::tls::wrap(&cfg.fabric_dir, &pairs)\` read ONCE
  when tcp-lan is built (D5: return a pass-through wrap on every hub that reads the live PairTable, else a first non-private pairing gets TLS only after a
  restart). \`N/fabric/sealer.rs\`: \`IdentitySealer::carrier_topic_x25519(&peer_enc_pk) -> Result<Zeroizing<Key32>, SealerError>\` (on no API; D11 feeds it
  to carrier::ntfy::topic_key), \`stored_hub_hint(&fabric_dir)\`. \`N/relay/tls.rs\`: \`pub fn wrap(fabric_dir: &Path, pairs: &PairTable) -> Option<Arc<dyn
  StreamWrap>>\` (the None stub D5 replaces, same signature).
- mesh111 fabric, the fix wave (M4b, 6516032): wholeness is RECORDED: \`PairedHub { node_id, role, via, ours: bool, theirs: bool }\` with
  \`PairedHub::whole()\` = ours && theirs (\`valid()\` unchanged); \`Fabric::offer_pair(address: Option<SocketAddr>, role: PairRole, via: PairVia) ->
  Result<OfferView>\` mints a PAIRING code (OfferView.bundle is None: the 111PAIR bundle is the daemon's, D6; one open offer at a time, member or
  pairing); the far hub's ENROLL that redeems it records \`theirs\` and pins its card at oob (never a member); \`Fabric::pair(...)\` (same signature)
  records \`ours\` on its ENROLL_OK and returns the record read back. A member code never records a pair; a recorded pair presenting a member code,
  the home hub presenting a pairing code, a role/via mismatch with an existing record are refused before the code is spent. Every forward path
  admits a pair only when whole (and not broken this session); a half pair's put meets REJECT_UNENROLLED; the daemon learns a pair became whole
  from FabricEvent::Settings (the two bools in paired_hubs); \`unpair\` drops the record only (the far hub's contact stays as it is). The fabric's
  pairing dial is kept only while the pair is half and released once whole (the daemon's step-5 rule holds a tcp pair's one connection: dial
  WHOLE pairs only, M5b). Every pairing and forwarding path is gated on \`hub_role\` (with it off the records stay, the rows wait in the store, the
  view shows whole = false, reachable = false, pending = 0). The forwarder does bounded work per trigger (per-target queues, dirty-row
  retargeting, O(1) untrack via the row's pkt_ref); \`pub const FORWARD_RESIDENT: usize = 4_096\` resident rows, the rest paged from the store in
  custody order (PairView.pending counts resident rows; Store::forward_stats has the totals); REJECT_FULL retries release one window per trigger;
  re-post rule (1) is applied at the next \`forward_take\` after a far frame; a pair's tcp CUSTODY_ACK is read only when fresh, attributed and from
  the pair the put went to; \`carrier_inbound\`/\`forward_take\`/\`forward_repost\` accept only whole carrier pairs. Settings-file keys (M4b in
  N/fabric/settings.rs): \`paired_hubs[].ours\` and \`.theirs\` (JSON bools after \`via\`, always written, false when absent).
  \`#[doc(hidden)] Fabric::forward_target_computations() -> Result<u64>\` is a test counter. D6: use offer_pair for the pair offer route and refuse
  a pair() whose code did not come from a 111PAIR bundle (the ENROLL_OK cannot tell a member code from a pairing code: a recorded limitation).
- node111 fabric seams, the fix wave (M5b, 0f822e5): \`sealer::StoredHubHint::{NoAnnounce, NoHub, Hub([u8; 8])}\`, \`IdentitySealer::stored_hub_hint(&fabric_dir)
  -> StoredHubHint\`; \`settings::rebuild_home_hub(enrolled, hint, announced: StoredHubHint, pairs)\` (a valid stored announce is authoritative: NoHub ⇒
  nothing rebuilt; Hub(a) ⇒ the pick must match; the cursor rule only without a stored announce); \`settings::pair_address_refusal(&Persisted, &PairLink)
  -> Option<&'static str>\` (an address the home hub, a peer or another pair uses is refused; \`set_pair_link\` calls it); \`pair_dials\` dials WHOLE
  tcp pairs only (a half pair's second ENROLL rides the fabric's own pairing dial); the pair halves are published to the PairTable and written to
  the file in one order under \`FabricHandle.persist\` (a tokio mutex; the bytes are built under the std lock, no await there), a half is dropped only
  for a record absent from the fabric's CURRENT settings (re-read when an incoming Settings lacks a record this handle holds); the pump's lag rule:
  after a broadcast lag the lost events are NOT replayed: the daemon drops the batch's Settings, re-reads the settings (sync_settings) and the tap
  sees exactly one FabricEvent::Fabric meaning "re-read everything" (D11: forward_take per carrier pair; D2/D3: re-read the status and the pair
  table); the tap runs after the batch's last Settings is applied, so pair_table() and persisted() read the batch's state; \`relay::tap(&FabricEvent)\`
  is a free function with no handle: D2, D3 and D11 keep their on/off and per-consumer state in process statics (OnceLock/atomics) read at no cost
  while off, unless D10 adds a handle in ApiState ext; \`replace_home_hub_address\` refuses an address already connected (Link::peers) and a peer's
  or a pair's address. For D6: set \`PairLink.far_holds_address = true\` only when the far hub will actually record this hub's address (the address
  in this hub's bundle passes the far hub's dial_refusal and is reachable from its network); otherwise leave it false (this hub then dials whenever
  it holds the far address): a private LAN address shown to a hub on another network with far_holds_address = true makes the far hub dial forever
  while this hub waits. A settings row written before M4b (ours/theirs absent) reads as a half pair.
- node111 §7.7 as built after M5c (db7ee22, bfbb33d): \`Rehomed { hub, from, to, marker: Option<u64>, void }\` (the marker is the hub contact row's
  \`first_seen_seq\`); \`FabricHandle::replace_home_hub_address(address, wait) -> Result<bool, FabricError>\` (D8's call): \`Ok(true)\` once a
  handshake at the address named the hub (or the fabric's own copy moved there meanwhile), \`Ok(false)\` when no handshake named it in time or the
  membership changed during the wait, \`Err(Invalid)\` for no home hub, a refused address (dial_refusal), a peer's or a pair's address, or one with
  a connection up already; the trial dial is released on every failure unless a pair dial or the fabric's own copy holds the address; the override
  is voided by a Settings showing the hub at neither address, a Contact Added/Removed for the hub, or after a pump lag a gone contact row or a
  changed marker; no void path releases the shown home-hub address. RESIDUAL FOR D8: a dial target only the fabric holds at the trial address is
  invisible to the daemon (no mesh111 \`Dialer::is_target\` yet; a foreign connection there can pass the trial's connected check, which sees no
  NodeId): do not call replace_home_hub_address while a pairing dial may be in flight at that address (read pair_table() and the pairing records
  first) and record in notes where that is not enough. \`settings::fold_dropped_records(&mut Persisted, &[PairedHub]) -> bool\`; set_pair_link
  folds a pending unpair into the records before storing a new half, so D6 may call Fabric::unpair and then set_pair_link at once.
- mesh111 forwarder after M4c (19df20b): the resident window is shared fairly (a swap probe evicts a down target's newest idle rows for an up
  target's stored rows; \`Store::forward_pending_lens\` reads row heads without their bytes); re-post rule (1) re-arms once per far-hub round
  (\`Slot::Heard\`). The public API is unchanged (\`forward_take\`, \`forward_done\`, \`forward_repost\`, \`carrier_inbound\`, \`PairView\`): nothing
  in Wave 1 depends on the forwarder's internals. M4d (a bounce: the probe's self-wake and its resets) is the next mesh111 task; if the lead runs
  it beside Wave 1 (a worktree) and lands it on M/fabric.rs mid-wave, a node111 build that fails ONLY with errors inside mesh111 is that landing
  in progress: wait 90 s and retry (up to five times) before reporting blocked-by: M4d.
`

const CTX = `
You are a BUILD subagent for the 111network OS, Layer 7 (Relay & Presence). Today is 2026-11-11. Repo root: ${ROOT} (a git repo on branch
claude/confident-ramanujan-y5ob49; you NEVER run git commit/add/stash/checkout/reset: the lead reviews and commits each task). You will not be
messaged mid-task; everything you need is in this brief and on disk.

THE LAW: the locked spec ${ROOT}/7-relay/README.md (v1.0, D32; §0 gives the path abbreviations M/ N/ NI/ ID/ A/ UI/ API RS/ LC/ L5:) and your task
row in ${ROOT}/7-relay/TASKS.md (find the row "| <id> |" in the Wave 1 table and read it whole: Task, Criteria, Check, Files (owned), the "never
adds" clause; the board's header paragraph and box rules bind you too; the ✅ rows of Wave 0 carry as-built facts you build on). Read your row
first, then the spec sections the row names (read with sed -n ranges; the spec is 1,484 lines), then the as-built code you build on (the row cites
file:line from 2026-11-01; re-check every line before editing). The contract ${ROOT}/4-interface/api/openapi.yaml is the binding form of every
route and shape (C1 landed it): a route answers exactly its shape and codes. Decisions: ${ROOT}/plan/DECISIONS.md D30, D31, D32. Do not re-open
decisions; implement them. If the spec is ambiguous, pick the reading that is safest (fail closed, lighter on the Pi floor) and record it under
"deviations". Owner rule in AGENTS.md: the Raspberry Pi is the floor: add no resident process, poll, timer, listener or runtime dependency to a
non-hub node; your row's "never adds" clause is an acceptance criterion.
${WAVE0_FACTS}
FILE OWNERSHIP IS STRICT: create or modify ONLY the files your row's Files column lists (plus new files inside dirs it names; "(T0 stub)" means the
file exists as a stub from the scaffold: fill it, remove its #![allow(dead_code)] if nothing is left unused). Other tasks run in parallel on other
files right now; if you need a change elsewhere, do NOT make it: describe it under "deviations" and continue (exception: a tiny additive pub mod /
re-export / registration line a new module needs, or the one call that wires your module into N/relay/mod.rs's start() or tap(): do it and list it;
N/relay/mod.rs belongs to D10, which runs last and merges). Never touch plan/, TASKS.md boards, INDEX.md or the spec.

HOUSE RULES: Rust edition 2021, typed errors (never panic on input; sizes checked before parse), doc comments citing the spec section ("§x.y"),
unit tests in-module and integration tests under tests/, rustfmt + clippy -D warnings clean (the workspace runs clippy --all-targets -D warnings in
CI); no new dependency (mesh111's and id111's sets are final; node111's policy is in ${ROOT}/1-bootstrap/TASKS.md "Dependency policy"; tokio-rustls
and rustls are D5's only); no wall clock in any ordering or cryptographic decision; nothing downloads at runtime; no Reticulum code; foreign bytes
are untrusted everywhere; every route: problem+json per ${ROOT}/4-interface/api/README.md, the agent's bearer refused (401) on every Layer 7 write,
loopback-only where the contract says; never a model name or identifier in any file; no attribution lines; never commit. UI tasks: TypeScript
strict, the lazy-chunk rule (${ROOT}/4-interface/README.md budgets; pnpm budget must stay green), every peer string through cleanPeerText, no
secret rendered, vitest tests beside the existing ones.

BOX: 4 CPUs, 15 GB RAM, a fixed disk allowance (about 25 GB free; do NOT cargo clean, do NOT create another target dir). Shared WARM target dirs,
one per workspace, which cargo's own lock serializes between agents: 1-bootstrap (node111) → CARGO_TARGET_DIR=${SP}/target/node111 · 5-messaging
(mesh111) → ${SP}/target/mesh111 · 2-identity → ${SP}/target/id111 · 3-intelligence → ${SP}/target/agent111. Always export the right one with
CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=2 and TMPDIR unset before any cargo command, from that workspace's directory. Iterate with narrow commands
(cargo test -p <crate> --lib <module>, cargo check) and run the row's full check once at the end. A build that fails only because of a file another
task owns (another agent is mid-edit; node111 compiles mesh111 and id111 from disk): wait 90 s, retry twice, then report "blocked-by: <task>" and
never touch that file. Toolchain: cargo 1.97 stable (rustfmt, clippy); no musl cross toolchains (musl stages SKIP: say so, never fake them); node 22 +
pnpm (UI deps installed in 4-interface/ui; pnpm build writes dist/ which is gitignored: build into a scratch outDir or delete dist/ after); python
3.13; shellcheck at /usr/local/bin/shellcheck; minisign at /root/.local/bin/minisign; jq, curl, strace present; unprivileged netns work (unshare
-Urnm) and iproute2 (ip) is installed; the proxy denies most live services. Scratch files under ${SP}/<task-id>/ only. Linux hygiene: nothing of yours
stays running when you return (ps and ss check), kill what you started, delete your scratch.

RUN YOUR CHECK (the row's Check column, refined in your brief) until it is green; paste its tail.

RETURN (structured): task id, status (done | partial | blocked), files changed or created (exact paths), the check command and its tail (at most
25 lines), deviations (each with why), notes for the reviewer (what to read first, open questions, and the exact names other rows must use).
Factual; the lead reads diffs.
`

const T = {
  D5: { model: 'opus', effort: 'xhigh', brief: `TASK D5 (row "| D5 |"): TLS 1.3 raw public keys for paired-hub links (§4.1-§4.5, §4.7; §14 rule 7; D31 Q1). Implement M6's mesh111 StreamWrap (read M/link/tcp_lan.rs as it now is: outbound(addr, expected: Option<NodeId>, TcpStream), inbound(remote, TcpStream) -> (BoxedStream, Option<NodeId>)) in N/relay/tls.rs with tokio-rustls: inbound peeks the first byte (0x16 ⇒ TLS, 0x31 ⇒ plain as today; peek without consuming: TcpStream::peek), TLS 1.3 only, Ed25519 raw public keys (RFC 7250) on both sides, client auth mandatory, the SPKI compared byte-exact against the pinned pairs then verify_tls13_signature_with_raw_key, no X.509, no CA, no SNI, no name check; inbound returns the pair whose pinned SPKI matched; an unpinned client key closes before any 111 byte; outbound: TLS to a non-private pair, plain to a private or lan = true pair, refused plain to a non-private pair unless its warning_hash is recorded (the pair table is M5's fabric/settings paired_hubs; read its as-built shape; expose a small trait or closure the host passes in so tls.rs holds no settings of its own). N/relay/linkkey.rs: a fresh Ed25519 key from the OS CSPRNG at the first pairing, secrets/link-key 0600 inside the 0700 secrets/ (find how secrets/ is laid out and permission-checked in NI/ and N/identity), never seed-derived, on no API, excluded from the backup set (say where the backup set is defined and that you did not touch it: a deviation if it needs a line), re-minted after a restore; its 44 B SPKI (DER SubjectPublicKeyInfo of an Ed25519 key) and the base32 form for the bundle. Dependencies: node111/Cargo.toml gains direct rustls (default features off; features ring, logging, std, tls12 exactly as ureq's selection so no second copy) and tokio-rustls 0.26.6 (default features off, ring only); run cargo tree -e features -p node111 and prove no aws-lc-rs and that the lock gains one entry (tokio-rustls) only; if the registry lacks 0.26.6 say which version the lock resolved and why. Never adds: runtime weight before a non-LAN pair (nothing constructed while no pair exists). Record: the 111 size delta (a release build is too heavy for this box: say so and give the debug delta) and the RSS of one pinned TLS session in your notes. Tests: tests/relay_tls.rs (new): the pinned handshake end to end through M6's seam over loopback with two TcpLan instances; a wrong client key, wrong server key, an X.509 certificate, TLS 1.2 and a non-Ed25519 key each refused before any 111 byte; a mismatched first announce refused; plain members served on the same listener. Check: CARGO_TARGET_DIR=${SP}/target/node111 from ${ROOT}/1-bootstrap: cargo test -p node111 --test relay_tls; cargo test -p node111 --lib relay::tls --lib relay::linkkey; cargo tree -e features -p node111 | grep -c aws-lc (must be 0); cargo clippy -p node111 --all-targets -- -D warnings.` },
  D4: { model: 'opus', effort: 'high', brief: `TASK D4 (row "| D4 |"): netlink address and route events plus the Termux fallback (§3.2, §3.3 bullets 1-3, §3.4; S15; §19 D8). NetlinkEvents::new takes the group mask (N/transport/netlink.rs); the spine passes RTMGRP_LINK and keeps its debounced instance and on_link_change untouched (N/transport/mod.rs: a DHCP renew never re-renders rnsd: prove it with the existing transport_spine test still green plus one new assertion). N/relay/netwatch.rs (T0 stub): a second socket with RTMGRP_LINK | IPV4_IFADDR | IPV4_ROUTE | IPV6_IFADDR | IPV6_ROUTE opened only while a consumer is subscribed (a subscribe/unsubscribe API with a count: the first subscriber opens, the last closes; consumers are D8's mdns-sd re-arm and D11's carrier), its own 2 s debounce so a route burst is one event, a failed bind logged once and reported to the subscriber; the Termux fallback: a coarse poll every gateway.termux_poll_min (default 15, min 9) only on a Termux hub with a carrier whose bind failed (the Termux detection exists in node111: find it; the poll reads what? the address list via the same netlink dump or /proc/net/route: pick the lighter, document it); no other node polls. Never adds: a socket, task or timer without a consumer; with neither a carrier nor the mdns-sd daemon the Layer 6 socket set is exactly unchanged. Record in notes: netlink sockets per configuration (none / mdns-sd on / carrier on / both). Tests: --lib transport::netlink (the mask), --lib relay::netwatch (subscribe opens, unsubscribe closes, a burst is one event, the bind failure path with a fake), --test transport_spine (unchanged behaviour). Check: CARGO_TARGET_DIR=${SP}/target/node111 from ${ROOT}/1-bootstrap: cargo test -p node111 --lib transport::netlink --lib relay::netwatch; cargo test -p node111 --test transport_spine; cargo clippy -p node111 --all-targets -- -D warnings.` },
  D9: { model: 'opus', effort: 'high', brief: `TASK D9 (row "| D9 |"): role bits 3-5 and the offer's gateway fact (§7.2, §7.5; §19 code seam 4). Bit 3 ROLE_GATEWAY (discovery.public_gateway_bit, after the gateway_bit consent: D10 writes the setting; you read it), bit 4 ROLE_OFFLOAD (discovery.offload_bit), bit 5 ROLE_ORIGIN (origin.serve) and the name map for NearbyNode.roles (bits → "gateway", "offload", "origin" beside the existing names), defined beside appdata in N/fabric/sealer.rs and set in roles(s) in N/fabric/mod.rs; the settings come from D1's N/relay/settings.rs (landed: read its view type; if its accessor names differ from the row, use them and say so); no mesh111 constant; appdata stays [1][roles u16 LE][0] plus the optional hint; a change re-mints the announce once (one fsynced counter value: read how the announce counter is persisted and prove one re-mint per change with a test). The hub offer JSON (N/dashboard/api_v1/fabric.rs) gains gateway {carrier, mode} (additive; from D1's settings when the carrier is on: carrier kind and mode; absent or null otherwise, per the contract's HubOffer.gateway); the 111HUB: text bundle unchanged (build watch: say in notes that the fact reaches the offer screen only). M5 (Wave 0) touched N/fabric/{settings,mod,sealer}.rs: read them as they now are and change only what the row names. Never adds: a re-mint without a change. Check: CARGO_TARGET_DIR=${SP}/target/node111 from ${ROOT}/1-bootstrap: cargo test -p node111 --lib fabric (roles_follow_settings extended) --lib dashboard::api_v1::fabric; cargo clippy -p node111 --all-targets -- -D warnings.` },
  D3: { model: 'opus', effort: 'high', brief: `TASK D3 (row "| D3 |"): the WAN probe removed, the gateway service row, the tick trim (§3.1, §3.2, §3.5, §3.6, §3.8; D30 Q1, Q14). Delete the WAN loop (N/services/probe.rs: the loop, WAN_INTERVAL, wan_probe); keep wan_probe_urls for NODE111_PROBE_204 (used by the installer's one-shot probe: read who calls what before deleting; the installer's probe stays). The gateway row keeps its id; its reason codes become gateway_none (state unset: no gateway configured), carrier_ok (live), carrier_pending (unset: the last check failed or items wait), gateway_off (unset: off by choice); wan_ok/wan_unknown/wan_offline retire everywhere in node111 (grep); detail = GatewayDetail {carrier, endpoint, mode, checked_ago_s, last_flush_ago_s, pending, error} per the contract, from N/relay/gateway.rs (T0 stub: fill it: a small state holder with a monotonic Instant for each age, fed by D11's carrier later and by the paired-hub carrier fact now: a non-private paired hub's reachable from M4/M5's fabric settings and events; the badge's four states per §3.8, never "online", never a member's view of its hub); on_event(&FabricEvent) consumes what it needs and returns at once while nothing is configured. The catalogue entry and requires copy verbatim (N/services/mod.rs). The tick trim: the Meshtastic monitor's 2 s tick parks on the devices kick while no binding is in interfaces.toml; the panel revocations and the panels reconcile park on a new state-change Notify beside snapshot() while no session or declared service needs them; every other tick untouched; measure idle wakeups/min before and after on this box (strace -c or /proc/<pid>/sched on a daemon root: say how) and put both numbers in notes. Never contacts install.111.network; never adds a timer or request. L2 (Wave 0) added check_internet: true to tests/api_v1_status_services.rs's UpdaterConfig literal: keep it. Check: CARGO_TARGET_DIR=${SP}/target/node111 from ${ROOT}/1-bootstrap: cargo test -p node111 --lib services --lib meshtastic --lib panels --lib relay::gateway; cargo test -p node111 --test api_v1_status_services (the four codes replace the wan_* cases); cargo clippy -p node111 --all-targets -- -D warnings.` },
  D7: { model: 'opus', effort: 'high', brief: `TASK D7 (row "| D7 |"): the hub as update origin, the serving side (§9.1; §19 C6, D7). GET /origin/v1/channel-111-stable.toml, /origin/v1/channel-111-stable.toml.minisig, /origin/v1/blob/{sha256} on the existing dashboard listener (no third port), mounted beside /health outside the session gate (N/dashboard/routes.rs: the open set; GET only, 405 otherwise; no CSRF; the same CSP; the Host guard admits the hub's own addresses), served only while hub_role (fabric settings), LAN sharing and origin.serve (D1's settings view) are all on, else 404, on every LAN-bound listener. N/origin/mod.rs (T0 stub): state/origin/ holds only bytes that passed ChannelManifest::parse_verified (L1's) and blobs whose sha256 and size match a pin in that manifest; the own target triple by default, others only from origin.targets; bodies stream from disk (tokio::fs + a body stream, never a resident buffer); admit(manifest_bytes, sig, blobs) for L4 (Wave 2: the 111 update --from / the daemon's own staged payload hand-over) and bytes() for RelayStatus.origin.bytes (a directory walk on demand, cached per change, never a timer). Never adds: a listener, timer or resident buffer. Record disk per served target in notes. Tests: tests/origin_serve.rs (new): 404 while off (each of the three switches), 405 on POST, served without a session, the Host guard, a missing blob 404, unverified bytes refused by admit, the manifest and its .minisig served byte-exact; use the dashboard test harness the other api_v1_*.rs tests use (serve_on). Check: CARGO_TARGET_DIR=${SP}/target/node111 from ${ROOT}/1-bootstrap: cargo test -p node111 --test origin_serve --lib origin; cargo clippy -p node111 --all-targets -- -D warnings.` },
  U1: { model: 'sonnet', effort: 'high', brief: `TASK U1 (row "| U1 |"): the Nearby radar (§10.1-§10.4; §2.3). In ${ROOT}/4-interface/ui: an own SVG radar component in the lazy nearby chunk replacing the ASCII placeholder (UI/views/nearby/radar.ts) and its line in index.tsx; the model (model.ts) computes rings (hops + 1; a LAN peer and hops_away = 0 on ring 1; a contact heard only through the hub on ring 2; unknown hops on the outer ring), the order within a ring (stronger SNR inside, else the more recent heard_ago_s), the angle (only when both ends report a position, else a stable hash of the id), every Meshtastic position as the coarsest uncertainty cell (10 bits, 46.7 km), the glyph by source kind, the legend verbatim from §10.1 rule 6; each dot's row shows its evidence (nodeEvidence from UI/lib/api/nearby.ts) and age (heardAgoNow + ageLabel with the page's clock between frames; no polling, a 1 s re-render timer only while the view is mounted is acceptable if the existing views do the same: check, else none). Buttons: "Look for 111 nodes here" (getRelay().browseRegistry()), on a member with a home hub "Find my hub" (the same call; the home hub comes from the store's fabric slice), the hop read (getRelay().nearbyHops(), a human), the link to the hosted Meshtastic panel's map (the existing panel link helper); no position setting. Use the mock (adapter.mock.relay / relayControl) in tests. Keep the chunk within the budget: pnpm build into a scratch outDir then pnpm budget (read package.json for how budget finds dist; if it needs dist/, build there and delete it after). Tests: test/layer7-radar.test.ts (new): rings, order, angle rule, cells, legend, "Find my hub" shown only with a home hub and sending exactly one browse. Check: pnpm typecheck; pnpm test; pnpm build && pnpm budget (paste the budget line); nothing left running.` },
  U3: { model: 'sonnet', effort: 'high', brief: `TASK U3 (row "| U3 |"): the fabric view, the Overview badge, the gateway row, the forwarding copy (§3.5 item 4, §3.8, §5.2, §5.6, §5.8, §13.1). In ${ROOT}/4-interface/ui: the mock fixture's gateway service row moves to the new codes and GatewayDetail (UI/lib/api/mock/fixtures.ts: wan_ok, interval_sec, last_probe go; the fixture validator must pass); the badge's four states (gateway_none, carrier_ok, carrier_pending, gateway_off) on Overview and on the gateway row, never "online", with the "metered cap reached" line beside them from the relay slice (gateway.metered_cap_reached; the store's relay slice is null until loaded: load it through getRelay() when the Overview or the services view mounts? NO: the row says the views read the slice; call apiStore.refreshRelay() on mount of the fabric view only, and let Overview show the badge from the gateway service row's reason_code, which every node has); the fabric view: per paired hub reachable, pending, reposts and the link state (tls / lan / plain (warned)); forward.refused (this session's count) and "too large for the carrier" (forward.too_large) with the 2,937 B bound in the copy; Messages: reason forwarding renders "Your hub holds it and passes it to their hub." and no copy says the dest's hub was found. Tests: test/layer7-views.test.ts (new): the four states, the metered line, the forwarding copy, the fixture validates. Check: pnpm typecheck; pnpm test; pnpm build && pnpm budget (scratch outDir or delete dist/ after); nothing left running.` },
  K1: { model: 'sonnet', effort: 'medium', brief: `TASK K1 (row "| K1 |"): the Layer 2 spec text (§18 L2 of the Layer 7 spec; §4.2, §7.2, §8, §11.2 there). Edit ${ROOT}/2-identity/README.md only, additively, in the places §18 L2 names: transport_id 0x04 "111-link-ed25519" reserved (unassigned bytes still fail closed); the domain tags 111/carrier-topic/v1, 111/epoch/v1, 111/broadcast/v1; the epoch unit (a day of the issuer's powered-on time) and source (hub-distributed, floored by the release), closing the §13 hand-off, with the 72 B wire and 104 B transferable forms and "not evaluated in v1"; the attestation carriage rule (§11.2); appdata bits 3-5 ("Layer 7 appends", node111 only); the topic derivation behind the identity handle. No code: v1 builds nothing for the epoch; say so in the text where §8.5 of the Layer 7 spec says it. Match the file's voice and section numbering (read it whole first; it is locked v2: add a dated "Layer 7 amendments (D30-D32)" note beside its status line rather than rewording locked rules). No model names. Check: git diff --stat names one file; paste the full git diff in your notes for the lead to read against §18 L2.` },
  K2: { model: 'sonnet', effort: 'high', brief: `TASK K2 (row "| K2 |"): cross-layer text and the reach doc (§18 L1, L3, L4, L6; §19 items 1, 2, 6, 8, 9, 10 (its MeshCore clause is K3's, skip it), C14; §6.6 item 3). Files: ${ROOT}/1-bootstrap/README.md (the /origin/v1/* routes beside /health in the machine contracts, the two verbs 111 update --from and 111 fabric pair, state/origin/, state/relay.json, state/staging.lock, secrets/link-key, the sentences §18 L1 names and the seams line, as §18 words them; it is locked v2.2: a dated "Layer 7 amendments (D30-D32)" note beside the status line plus the additive lines); ${ROOT}/3-intelligence/README.md (item 6: the three tools and nearby.read's growth, one additive paragraph); ${ROOT}/README.md (§7 heartbeats per item 1; "Category 5" per C14; §6 "join them in sequence" per item 10: user-facing and general, no code names); ${ROOT}/6-transport/README.md (a Layer 7 amendments note: the netlink mask and second socket, the WAN loop removed, the gateway row, the parked ticks, mdns-sd on events, [order] as preference, receipt.gateway an install fact the daemon never reads, G7 plus idle.sh, rnpath hops through the human-only route, the rns heartbeat stays Layer 6's); ${ROOT}/4-interface/README.md (/settings/l7, the radar, the D21 map amendment recorded for v1.1 per §10.5); ${ROOT}/plan/cross-layer/parked.md (P5 closed, item 9); ${ROOT}/7-relay/docs/tunnel.md (new: WireGuard or Headscale, never required; a tunnel pair is marked lan = true; a stock Tailscale client logs to its vendor unless TS_NO_LOGS_NO_SUPPORT; short, factual, no vendor praise). Read each spec line (§18, §19) before writing its sentence; quote the spec's words where it gives them. No model names. Check: git diff --stat names only these files; paste the full git diff in your notes for the lead to read line by line.` },
  D4b: { model: 'opus', effort: 'high', brief: `TASK D4b (row "| D4b |"): the review's findings on D4 and D9, both LANDED (read rows "| D4 |" and "| D9 |" of the board with their
as-built facts, then N/relay/netwatch.rs, N/transport/{mod,netlink}.rs, tests/transport_spine.rs and the relay-roles section of N/fabric/mod.rs
as they now are). Four small fixes, each with a test that fails without it:
(1) netwatch.rs reconcile_poll: a Termux poll restart (the subscribed minimum period changed, or a second carrier subscribed) re-reads the route
    fingerprint as its baseline and starts the period over, so a route change that happened since the old task's last tick is never delivered.
    FIX: the last delivered fingerprint lives in the watch's state and is handed to the restarted task (the period may restart; the change may
    not be lost). Test: change the scripted routes, restart the poll through a second subscription with another period, the next tick delivers
    one Poll.
(2) tests/transport_spine.rs an_address_change_never_re_renders_rnsd cannot fail: it runs a hand copy of transport::start's wiring, so nothing
    checks that start() binds LINK_WATCH_GROUPS. FIX: the spine's source construction moves into one function that start() calls (for example
    transport::link_watch_source() -> the NetlinkEvents it opens with LINK_WATCH_GROUPS), and the test calls that same function and asserts
    groups() == RTMGRP_LINK on the socket it returned (skip with a printed reason where a sandbox refuses the bind, as the existing socket
    tests do). The rest of the test stays.
(3) netwatch.rs: every datagram on the second socket counts as a change. With RTMGRP_IPV6_IFADDR bound, a Router Advertisement that only
    refreshes a SLAAC address's lifetimes sends RTM_NEWADDR, so on an IPv6 LAN each RA would become one NetEvent::Changed: one carrier request
    (D11) and one mdns-sd re-arm (D8) with nothing changed, all day, against the Pi-floor figure "idle outbound 0"; and RAs closer than the
    debounce would starve the event. FIRST verify what the kernel sends (read the netlink messages for real in a network namespace if you can:
    unshare -Urn, add an address, change only its lifetimes with ip addr change ... valid_lft/preferred_lft, and see whether a datagram arrives;
    report what you saw; if the box cannot show it, say so and keep the fix, which is cheap either way). FIX: before delivering Changed, the
    watch compares a fingerprint of what its consumers care about and delivers only when it moved: the set of (interface index, family,
    address, prefix length) and the routes. Take the addresses from the netlink messages themselves if you parse them (RTM_NEWADDR / RTM_DELADDR
    / RTM_NEWROUTE / RTM_DELROUTE headers: sizes checked before every read, an unknown or short message counts as a change, never a panic), or
    from one bounded read after the debounce (getifaddrs through the libc the crate already links, plus the same capped /proc route reads the
    Termux poll uses); pick the lighter and say why. No timer, no poll, no task beyond what exists; nothing runs with no subscriber. The
    debounce must not starve: a burst that never goes quiet for 2 s still delivers within a bound (say 10 s from the first datagram) when the
    fingerprint moved. Tests: a lifetime-only refresh delivers nothing; an added address, a removed address, a changed default route each
    deliver one Changed; a continuous burst delivers within the bound.
(4) fabric/mod.rs set_relay_roles stores the new roles and returns true when the computed bits differ, but refresh_announce reports nothing:
    when the re-mint fails (the counter cannot be persisted: Stale, or the blocking task fails) the method still says true, and the next call
    with the same switches computes no difference and never tries again. FIX: refresh_announce returns its outcome; on a failed mint
    set_relay_roles puts the previous roles back (so the same call tries again) and returns an error or false as fits the callers (D10 will
    call set_relay_settings after every relay settings write and must be able to tell the owner the bit is not announced yet: give it a
    typed result, document it for D10 in the notes). Test: a mint that fails once (the test seam the file already has for the counter, or the
    smallest honest one), then the same call again mints once.
NEVER ADDS: a resident task, timer, poll, listener, file write or dependency; no socket without a subscriber. FILES: N/relay/netwatch.rs,
N/transport/mod.rs, N/transport/netlink.rs, N/tests/transport_spine.rs, N/fabric/mod.rs (the relay-roles section and refresh_announce only).
CHECK: cargo test -p node111 --lib transport::netlink; --lib relay::netwatch; --test transport_spine; --lib fabric; cargo clippy -p node111
--all-targets -- -D warnings; rustfmt --edition 2021 --check on your files (never cargo fmt -p node111: other rows edit the crate). Report the
netlink sockets and tasks per configuration again (unchanged is the goal) and what the namespace experiment showed.` },
  D4c: { model: 'opus', effort: 'high', brief: `TASK D4c (row "| D4c |"): the re-review of D4b (LANDED: read row "| D4b |" and N/relay/netwatch.rs, N/transport/netlink.rs,
the relay-roles section of N/fabric/mod.rs as they now are). Small, bounded fixes; each with a test that fails without it:
(1) The Termux poll fingerprints only /proc/net/route and /proc/net/ipv6_route. On Android the IPv4 main table is mostly empty (per-network
    policy tables), so a Wi-Fi / cellular switch may never move it. FIX: the poll compares the same change_fingerprint the netlink path uses
    (addresses and link state through getifaddrs, plus the routes): one bounded read per tick, nothing new resident.
(2) The netlink path drops a burst unless the fingerprint moved, and its IPv4 route part is the main table only, while the socket delivers
    route messages of every table: a default route changed in a policy table (a VPN, Tailscale's exit node, NetworkManager, Android) with
    unchanged addresses delivers nothing. FIX: the reader looks at each netlink message header in a datagram (nlmsghdr: length and type; sizes
    checked before every read; a short, unknown or unparsable message counts as a change; never a panic): a burst that carried any RTM_NEWROUTE
    or RTM_DELROUTE whose rtm_table is neither main (254) nor local (255), or any RTM_DELADDR, RTM_DELLINK, or an RTM_NEWLINK, counts as a
    change without asking the fingerprint; a burst of RTM_NEWADDR and main / local table route messages only is settled by the fingerprint as
    today (so a lifetime-only refresh still delivers nothing). Verify the message shapes for real in a namespace as D4b did (unshare -Urn; ip
    route add ... table 52; ip link set down / up) and report what arrived. State the remaining trade-off in the module doc: a change that
    happens and reverts inside one burst with only NEWADDR and main-table messages is not seen.
(3) reconcile_poll's stop path and close() clear poll_seen and abort the task without bumping poll_gen, so a tick already past its read can
    write poll_seen back after the stop. FIX: bump poll_gen wherever the poll is aborted.
(4) set_relay_roles: two overlapping calls can leave the handle and the announce disagreeing (the second returns Ok(false) while the first's
    re-mint is still in flight and then fails and reverts). FIX: the swap, the compare and the revert happen under the re-mint lock (the calls
    serialise), and "changed" is decided against what was last announced. Test with two concurrent calls, the first failing.
(5) netlink.rs a_burst_that_never_goes_quiet_still_fires_within_the_bound asserts against real time. FIX: tokio::time::pause() and advance(),
    or assert only what load cannot break.
NEVER ADDS: a task, a timer, a poll, a resident socket, a dependency. FILES: N/relay/netwatch.rs, N/transport/netlink.rs, N/fabric/mod.rs (the
relay-roles section only). CHECK: cargo test -p node111 --lib transport::netlink; --lib relay::netwatch; --test transport_spine; --lib fabric;
cargo clippy -p node111 --all-targets -- -D warnings; rustfmt --edition 2021 --check on your files (never cargo fmt -p node111).` },
  K4: { model: 'sonnet', effort: 'high', brief: `TASK K4 (row "| K4 |" of the board: read it whole; it is a Wave 2 row run early because D5 has LANDED and changed
${ROOT}/1-bootstrap/Cargo.lock: tokio-rustls and rustls are now direct dependencies of node111). Do exactly the row: (1) ${ROOT}/LICENSING.md gains
the Layer 7 §4.5 row verbatim (read §4.5 of ${ROOT}/7-relay/README.md for the sentence) in the table where the other linked Rust crates are
recorded, and its §5 counts are RE-COUNTED from the lockfiles as they now are (read how the file says the counts were made and redo that count;
do not trust the row's expected numbers if the lock says otherwise: report the difference); ntfy is recorded as an owner-run server, never linked
or shipped (Apache-2.0 / GPL-2.0), in the section where external programs and services are listed; (2) ${ROOT}/1-bootstrap/TASKS.md: a "Layer 7
additions" sign-off row under "Dependency policy" for the direct rustls and tokio-rustls, in the shape of the Layer 5 and Layer 6 rows there
(the reviewer sign-off is the lead's: write "approved by the lead on the D5 review, 2026-11-12" and the features enabled, read from
${ROOT}/1-bootstrap/crates/node111/Cargo.toml: no aws-lc, ring only); (3) regenerate ${ROOT}/THIRD-PARTY-NOTICES.md with
python3 1-bootstrap/scripts/gen-notices.py (read the script's header first: what it reads, whether it needs the cargo registry sources, which
exist on this box under ~/.cargo/registry; never hand-edit the generated file). CHECK: python3 1-bootstrap/scripts/gen-notices.py --check; the
notices tests the repo has (python3 -m unittest discover -s 1-bootstrap/scripts/tests -p "test_*.py" and 1-bootstrap/scripts/notices/tests);
git diff --stat shows only your three files. No model name, no attribution lines, never commit.` },
  D3b: { model: 'opus', effort: 'high', brief: `TASK D3b (row "| D3b |"): the review's findings on D3 (LANDED: read row "| D3 |" with its as-built facts, then N/relay/gateway.rs,
N/services/{mod,probe}.rs, N/meshtastic/mod.rs, N/panels/mod.rs as they now are) and, in the same pass because they share N/fabric/mod.rs, every
item of row D4c (its brief follows this one in full). Each fix gets a test that fails without it.
 G1 (medium) gateway.rs internet_pairs counts any whole tcp pair not marked lan and never looks at its address; §4.1 rule 1 makes a pair at a
    private address (RFC 1918, RFC 6598, ULA, link-local) a LAN pair, as relay/tls.rs already does. FIX: a pair claims the internet only when
    it is not marked lan AND its address is known and not private (use the one is_private the TLS path uses: make it reachable from both, do not
    copy it); an unknown address claims nothing. Tests: 192.168/16, 100.64/10, a ULA, a public address, no address.
 G2 (medium) Gateway::configure keeps the facts it holds (answered, checked, error, last_flush, pending) when the carrier's endpoint, kind or
    consent changes while the gateway stays on, and reports them against the new endpoint. FIX: the carrier facts reset whenever the configured
    carrier differs; the pair facts reset when enabled flips.
 G3 (medium) the services registry's 2 s tick awaits gateway::refresh(), which awaits Fabric::status() (a round trip through the fabric actor
    with no timeout, and on a hub a read of every contact) on the tick task; probe.rs's own contract says the tick reads cached snapshots only.
    FIX: FabricHandle keeps the per-pair views (reachable, pending) it already computes when it handles a fabric event, and refresh() reads that
    cache without awaiting the actor; no status call is added anywhere. Test that refresh() makes no actor round trip (a counter or a handle
    whose actor is parked).
 G4 (low) §3.6: the Meshtastic panel revocations tick runs only while a Meshtastic panel session is open; the code also runs it whenever the
    meshtastic row is not unset. FIX: park on the session set (a Notify the session create and revoke fire) together with the revocation
    stream; if that cannot be done inside the row's files, say exactly why and record the wakeup cost instead.
 G5 (low) detail.mode for a paired-hub claim is "5B" even on a Termux hub, against the contract (5B: a held stream, never on a Termux hub).
    FIX: on Termux the paired-hub mode is "5A"; say so in the doc.
 G6 (low) no test drives the spawned loops (panels::spawn, attach_panels, the monitor task) to show they park and wake, and Gateway::refresh()
    has no test through a FabricHandle. FIX: paused-time tests for each loop (no pass while parked over N ticks; one pass after a change) and
    one test from a FabricHandle with an internet pair through refresh() to the row.
 ALSO from the review's gaps: trace whether a change of PairLink.lan or PairLink.address emits the fabric event that refreshes the gateway's
 pair set (fix or say); the error strings D11 will hand to record_check reach the services row's detail, which the agent's bearer can read:
 give record_check a documented rule that the caller passes a fixed reason and never a URL, and scrub anything that looks like one.
NEVER ADDS: a task, a timer, a poll, a listener, a status round trip on a tick. FILES: N/relay/gateway.rs, N/services/{mod,probe}.rs,
N/meshtastic/mod.rs, N/panels/mod.rs, N/fabric/mod.rs, N/relay/tls.rs (only to export is_private, no behaviour change), N/tests/api_v1_status_services.rs,
plus D4c's files (N/relay/netwatch.rs, N/transport/netlink.rs). CHECK: cargo test -p node111 --lib relay::gateway; --lib services; --lib
meshtastic; --lib panels; --test api_v1_status_services; D4c's checks; cargo clippy -p node111 --all-targets -- -D warnings; rustfmt --edition
2021 --check on your files (never cargo fmt -p node111). Report the idle wakeups per minute before and after as D3 measured them.` },
  D5b: { model: 'opus', effort: 'high', brief: `TASK D5b (row "| D5b |"): the reviews' findings on D5 and D7 (both LANDED: read rows "| D5 |" and "| D7 |" with their as-built
facts, then N/relay/{tls,linkkey}.rs, N/origin/mod.rs, N/dashboard/routes.rs, tests/{relay_tls,origin_serve}.rs as they now are). Each fix gets
a test that fails without it.
 TLS (D5):
 T1 tls() reads secrets/link-key (open, fstat, read, hex decode, key derivation) on EVERY handshake, inbound and outbound, although the module
    doc says it is built once and kept: any host that can reach the fabric port makes the hub do that work with one 0x16 byte. FIX: the loaded
    key and the built TLS state are cached and reloaded only when the key file changed (one fstat: dev, inode, mtime, length; or a generation
    that load_or_mint and discard bump): prove with a counter that N handshakes read the file once.
 T2 LEAD RULING on the peek: the peek stays whenever ANY pin exists, a lan pair's included (the far hub may not have marked the pair lan and
    then dials TLS; refusing it would break a valid pairing). Say so in the module doc as a declared exception to the row's "no weight before a
    non-LAN pair", with the cost after T1 (one peek per inbound connection, nothing built until a 0x16 arrives). With no pin at all nothing is
    peeked (the author's reading: kept).
 T3 the refusal tests (a non-Ed25519 key, an X.509 certificate, TLS 1.2) cannot fail if the specific check is removed, because the pin
    comparison refuses the same peer first. FIX: one test per check that isolates it (for example a verifier-level test that hands
    check_signature a non-Ed25519 scheme with a matching pin; an assertion on the specific rustls error for X.509 and for TLS 1.2), and a
    truthful comment where the old tests stay.
 T4 the link key must not survive an identity restore (§4.2): relay::linkkey::discard exists and is tested but nothing calls it. FIX: call it
    in NI/cli.rs identity_restore right after a successful restore (one call, its error logged and not fatal), with a test at the level that
    file's tests use.
 T5 loopback: D9's test sets lan = true on a loopback pair because the wrap refuses outbound plain to a non-private pair. LEAD RULING: loopback
    (127/8, ::1) is NOT private for §4.1 (the spec lists RFC 1918, RFC 6598, ULA, link-local); the test's lan = true stays. Add loopback to the
    is_private tests as an explicit "not private".
 T6 record the row's figures the review found missing: run the ignored memory_of_pinned_sessions measurement and the debug-build size delta
    again and put both in your notes for the board (musl release sizes stay SKIPPED on this box).
 ORIGIN (D7):
 O1 copy_hashed opens the source blob with a plain File::open (no O_NONBLOCK, no O_NOFOLLOW) while the write mutex is held: a FIFO blocks the
    origin. FIX: the same flags and the regular-file check the serving side uses, before the lock is taken or on the opened fd.
 O2 Origin::shared does get-then-insert in two steps, so two first callers can each build an Origin with its own write mutex. FIX: one Origin
    per process: build it once at daemon start beside the fabric handle's ext (N/daemon/mod.rs: one line) and have shared() only read it, or one
    get-or-insert under the ext's write lock (a small ApiState helper in N/dashboard/api_v1/mod.rs).
 O3 a failed second atomic_write (the signature) leaves the new manifest with the old signature for good. FIX: both files are written to
    temporary names first and renamed signature first, manifest second; a failure before the second rename leaves the previous pair served.
    Test with an injected failure.
 O4 a blob already present counts as held by its name and size and is never hashed, so bytes can be served unverified. FIX: admit re-hashes a
    held blob before it counts it (bounded by the targets of one manifest); a mismatch removes it and reports it missing.
 O5 AdmitError::Unverified carries the parser's error text (it can quote untrusted bytes) although the type is documented log-safe. FIX: a
    fixed reason per failure class; the detail only at debug level, length-capped and escaped.
 O6 no test streams a body over more than one 64 KiB frame. FIX: a unit test on stream_body with a file of several hundred KiB (frame count
    > 1, bytes exact) and a Range request across a frame boundary if the route serves ranges.
NEVER ADDS: a listener, a task, a timer, a dependency, a pub route. FILES: N/relay/{tls,linkkey}.rs, N/tests/relay_tls.rs, NI/cli.rs (the one
call and its test), N/origin/mod.rs, N/tests/origin_serve.rs, N/tests/fixtures/l7-D7/** (only if O6 needs a larger fixture: prefer a temp
file), and ONE of N/daemon/mod.rs or N/dashboard/api_v1/mod.rs for O2. Another task (D3b) edits N/relay/tls.rs ONLY to make is_private
reachable from gateway.rs (visibility, no behaviour): if your edit and its collide, keep both. CHECK: cargo test -p node111 --test relay_tls;
--lib -- relay::tls relay::linkkey; --test origin_serve; --lib origin; the test you add for NI/cli.rs; cargo tree -e features -p node111 | grep
-c aws-lc (0); cargo clippy -p node111 --all-targets -- -D warnings; rustfmt --edition 2021 --check on your files (never cargo fmt -p node111).` },
  D6: { model: 'opus', effort: 'xhigh', brief: `TASK D6 (row "| D6 |"): pairing (§4.6 steps 1-6; §4.1 rule 1 lan; §13.1 pair routes; D31 Q1, Q5). D5 has LANDED (read N/relay/{tls,linkkey}.rs: the SPKI, the pin table hook) and M5 (N/fabric/settings.rs paired_hubs: read its as-built shape; the lan mark and the warning hash live there per the row). The bundle 111PAIR:<card b32>:<code>:<link SPKI b32>:<address or empty>:<role>:<via> printed and parsed in node111 (N/relay/pair.rs, T0 stub): three fields from the left, two from the right, the address the rest (a bracketed IPv6 literal with a port works); 111HUB: untouched. Routes in N/dashboard/api_v1/pair.rs (T0 stub; register its ApiRouter where T0 merged it): POST /relay/pair/offer {role, via} → PairOffer {schema, bundle: Card, code, link_spki} (the Card type as the Layer 5 offer route builds it: text + QR), dismissed with the Layer 5 DELETE /fabric/hub/offer; POST /relay/pair {bundle, role, via, lan?, accept_warning?} → 202 PairedHub; the pick must be the complement of the bundle's role (peer↔peer, leaf↔gateway) with the same via, else 409 pair_role_mismatch and nothing recorded; carrier ⇒ peer only; leaf, gateway ⇒ tcp only; both routes 409 needs_hub without hub_role; loopback only (403 loopback_only); the agent's bearer 401. Each hub pins the other's SPKI, then runs ENROLL both ways (inside TLS when not private; the second ENROLL rides the first hub's connection; never over a carrier: the fabric's enrol call exists for members: find how the Layer 5 scan side enrols and reuse it through FabricHandle); the D30 Q21 warning verbatim as 409 consent_required, consent.action = pair_plain, subject the far NodeId, its hash kept with the pair; refusing pairs nothing. DELETE /relay/paired/{node_id} (loopback): forget the pair and its pinned key; forward rows held for it return to the other targets (M4's API: say which call). 111 fabric pair --show and 111 fabric pair <bundle> [--lan] in N/fabric/cli.rs (the CLI talks to the daemon's routes as the Layer 5 verbs do), usage lines in NI/cli.rs. Never adds: a dial or record on a non-hub. Tests: tests/relay_pair.rs (new): the bundle round trip incl. IPv6, the complement matrix, the via rules, needs_hub on both routes, the warning round trip, a lan = true pair on a TEST-NET address running plain with no pair_plain consent, unpair; --lib relay::pair. Check: CARGO_TARGET_DIR=${SP}/target/node111 from ${ROOT}/1-bootstrap: cargo test -p node111 --test relay_pair --lib relay::pair; cargo clippy -p node111 --all-targets -- -D warnings. LEAD FOLLOW-UP: 7-relay/docs/tunnel.md steps 2 and 4 say that no control is pinned yet for the offered address and that the lan mark is the PairRequest field only: after your CLI lands, update those two steps to name the real verbs as built and how the bundle's address is chosen as built; if the owner still cannot put a tunnel address into the bundle, keep the doc honest and record it under deviations as an owner-facing gap (a tunnel pairing needs the tunnel address in the bundle); touch nothing else in that file; list it under files.` },
  D8: { model: 'opus', effort: 'xhigh', brief: `TASK D8 (row "| D8 |"): the DNS-SD record, the mdns-sd re-arm, lost-hub recovery (§3.3 bullets 4-5, §7.1, §7.4, §7.6, §7.7; D31 Q4). D4 has LANDED (N/relay/netwatch.rs: subscribe to its debounced address events). THE FIRST TEST YOU WRITE: on a debounced address event, set_ip_check_interval(1), one one-shot 1,500 ms tokio::time::sleep, set_ip_check_interval(0), all in the one task that owns the mdns-sd handle (N/dashboard/mdns.rs); get_ip_check_interval() reads 0 within 2 s and the A records changed; 0 only after the second bind, and on a bind failure the 5 s check stays, logged once. A second ServiceInfo _111._tcp.local.: instance the L2 label, host the dashboard's mDNS host, SRV 11183 (the fabric listener's port: read it from the fabric settings), A only, TXT v=1, r=<4 hex roles>, k=<64 hex NodeId> (78 B), present only while discovery.lan_record (D1's settings), a LAN-bound dashboard and a LAN-bound fabric listener; never starts the daemon. N/registry/mod.rs (T0 stub) and N/dashboard/api_v1/registry.rs (T0 stub; register its ApiRouter where T0 merged it): GET /registry → {schema, rows: RegistryRow[]} and POST /registry/browse (a human; bearer 401; a browse of _111._tcp for at most 5 s then stop_browse; 409 lan_off while LAN sharing is off; 202 RelayAccepted), the SSE registry frame {rows} on a change; each row says which layer answered (answered_by hint | signed; a signed row from a contact's role bits 3-5 (D9's name map) heard this session), every string untrusted text; never an alias, load, battery, position or model. Lost hub: "Find my hub" is POST /registry/browse on a member with a home hub (no new route: every row of that browse is matched against the home hub's k), and an address event while the home hub's redial back-off sits at its 60 s maximum starts the same browse with no call ⇒ browse 5 s, match k, keep the address only after a handshake names the hub (M5's FabricHandle call: read its name); no /24 sweep; no browse on Termux. The sealed query stays unbuilt. Never adds: a listener (the record rides mdns-sd's socket), a browse timer. Record in notes the wakeups/min change where the bind succeeds (the 5 s ip check parked). Tests: --lib dashboard::mdns, --lib registry, tests/registry_mdns.rs (new: the record's presence rule and TXT, a browse stopped within 5 s, 409 lan_off, bearer 401 on the browse, a member's browse matching its home hub's k and keeping the address only after the handshake; a fake mdns-sd daemon or the real one on loopback as the Layer 4 mdns tests do: read them). Check: CARGO_TARGET_DIR=${SP}/target/node111 from ${ROOT}/1-bootstrap: cargo test -p node111 --lib dashboard::mdns --lib registry --test registry_mdns; cargo clippy -p node111 --all-targets -- -D warnings.` },
  D2: { model: 'opus', effort: 'xhigh', brief: `TASK D2 (row "| D2 |"): presence and the Nearby back end (§2.1-§2.5, §10.1 rule 1, §13.1 /nearby/hops; §19 code seam 2; D31 Q2). D9 has LANDED (the role name map for NearbyNode.roles). mesh111: Fabric::attributed_on(peer) -> Option<LinkKind> in M/fabric.rs (read-only; M4 (Wave 0) reshaped fabric.rs: read it as it now is; touch nothing else there; CARGO_TARGET_DIR=${SP}/target/mesh111 for that crate). node111 N/relay/presence.rs (T0 stub): facts (peer, transport, evidence, local_seq, heard_at: Instant) in memory for the session from M5's tap (relay::tap(&FabricEvent) → presence::on_event: read the FabricEvent variants M4/M5 landed): an inbound message and an own delivered ⇒ the 111 row, tcp-lan when attributed_on says tcp-lan else hub; hub.reachable ⇒ the hub's row, tcp-lan; a paired hub's ack or put ⇒ its row, the pair's via (tcp-lan or carrier); relayed, a card, an announce, a hello, a keepalive ⇒ nothing; evidence strings verbatim from §2.2. The 111 source "111 nodes heard by this node" in N/dashboard/api_v1/nearby.rs (C1 left kind_for("111") and EVIDENCE_TRANSPORTS; the bridge sink never admits 111: the presence module feeds the Nearby document through its own path): device_id null; a row only with a fact this session; id the 64-hex NodeId (is_hex32 expects 32 bytes = 64 hex: check and widen for this source only if needed), num 0, names from the alias or the L2 label, last_heard_hint null, heard_ago_s (from heard_at), heard_seq, evidence, roles; the 512-row cap, oldest forgotten; the nearby frame on a change only. The inert heartbeat validator (§2.4 item 3) as a pure function D10 calls: after the transport check (no transport takes a beat in v1), a duty-cycle band or a licensed board ⇒ 409 licensed_board, a phone under 9 min ⇒ 400 bad_request; unit tests mark a transport offered, a phone category, a licensed board. POST /nearby/hops (a human; bearer 401; register it in the nearby router): one rnpath -t -j through the rnstatus runner (N/transport/rnstatus.rs: its timeout and owner guard; add the rnpath spawn beside rnstatus, same guards), parsed as untrusted JSON into hops_away of the reticulum source only; 409 component_unavailable while the component is off; 429 rate_limited within 30 s; 202 RelayAccepted; never on a GET. Announces unchanged, no rate limit. Never adds: a timer refreshing ages, a task per contact, a stored byte, a spawn on a GET. Check: CARGO_TARGET_DIR=${SP}/target/mesh111 cargo test -p mesh111 attributed_on (from ${ROOT}/5-messaging); CARGO_TARGET_DIR=${SP}/target/node111 from ${ROOT}/1-bootstrap: cargo test -p node111 --lib relay::presence --lib dashboard::api_v1::nearby --lib transport::rnstatus; cargo clippy -p node111 --all-targets -- -D warnings. When you route POST /api/v1/nearby/hops, REMOVE "runNearbyHops" from the UNROUTED allowlist in N/tests/dashboard_auth.rs and run cargo test -p node111 --test dashboard_auth (the contract walk then expects 401 for the agent bearer and not 401 for trusted loopback or a paired session on it).` },
  A1: { model: 'opus', effort: 'high', brief: `TASK A1 (row "| A1 |"): the agent tools (§13.3; C1's pin items 1 and 2). Crate ${ROOT}/3-intelligence/crates/agent111 (A/ = src/; CARGO_TARGET_DIR=${SP}/target/agent111). Read A/src/tools.rs whole (the schemas beside nearby.read, the dispatch, how a tool calls the daemon and how untrusted text is fenced), A/tests/daemon_tools.rs and chat_loop.rs (the tool count assertions 17 → 20), and the contract (openapi.yaml "Layer 7": GET /relay = RelayStatus, GET /nearby with the 111 source, POST /proposals with kind relay). Tools: presence.read {peer?} → the 111 rows with their evidence inside a labelled untrusted block; gateway.read → RelayStatus (no secret can be in it: the contract has only {set: bool}; still never echo token/topic_secret keys); nearby.read gains the 111 source and, after a human's POST /nearby/hops, Reticulum hops (it never triggers a run); relay.propose {actions, why} → a kind: relay proposal through the existing proposal writer with the closed vocabulary (gateway.enable {carrier, mode}, hub.pair {}, presence.interval {transport, minutes}, carrier.flush {}; the body is one JSON object {why, actions}; reject any other action name before writing). Reads freely, writes nothing; never accepts a consent, enters a code or sees a secret; names and evidence reach the model only as untrusted text; an offload target only as the private IP literal guard_model_base admits, never from a TXT record. Tests: A/tests/layer7_tools.rs (new; the fake daemon the other tests use), daemon_tools.rs and chat_loop.rs updated to 20. Check: sh ${ROOT}/3-intelligence/scripts/check.sh (musl stages SKIP: say so); cargo test -p agent111 --test layer7_tools --test daemon_tools --test chat_loop. LEAD FOLLOW-UP: 3-intelligence/README.md, the paragraph 'Appendix — Layer 7 tools' (added by K2) says the three tools are specified and that row A1 has not landed ('the code still offers seventeen', 'until then the Layer 6 appendix's 17 holds'): when your check is green, update those status sentences to as-built (the three tools exist, the count pins are 20), touching nothing else in that file; list it under files.` },
  U2: { model: 'sonnet', effort: 'high', brief: `TASK U2 (row "| U2 |"): Settings → Layer 7 (§12; §2.4, §2.6, §4.6, §6.3, §9.3, §9.5 T4). In ${ROOT}/4-interface/ui: /settings/l7 "Relay & presence" as its own lazy chunk ≤ 64 KB (the /settings/l6 precedent in UI/views/settings/index.tsx), files under UI/views/settings/l7/ (new). Sections: Presence (the interval per transport from RelayStatus.presence.heartbeat, shown and inert: a write answers 409 no_such_transport or settings_locked and the page shows the problem's detail verbatim; the §2.6 airtime line; the bans); Gateway (on, carrier kind, server, mode, the consent, Check now, Flush now, the badge from gateway.state; the http:// token-in-clear note); Paired hubs (my bundle as QR and text from pairOffer, scan or paste theirs, the role and link pick with the complement rule explained, the lan mark, per pair tls rpk / lan / plain (warned), unpair); Discovery (the LAN record, bits 3-5 with their costs from §12); Updates (the origin order from updateOrigins(), the internet check switch, "serve updates for" (origin.serve + targets), the 111 update --from instruction, "newest release seen: <calver> from <origin>"); Metered networks (the list, add/remove by iface or ssid with a daily cap). Every consent dialog shows warning_text verbatim and repeats the call with accept_warning (relayConsentOf from UI/lib/api/relay.ts); secrets are write-only inputs (never prefilled; {set: true} shows "set"). Also: UI/lib/api/bridge.ts's own CONSENT_ACTIONS list drops the four Layer 7 actions: switch it to the generated CONSENT_ACTIONS tuple from types.gen.ts and widen the agent-view accept gate (proposals.tsx isBridgeKind) so a relay proposal runs the consent round-trip (C1 noted both; list these two files under files and say so in notes; keep the change minimal). The page reads the store's relay slice (apiStore.refreshRelay() on mount) and the mock answers everything (UI/lib/api/mock/relay.ts; relayControl for states). Tests: test/settings-l7.test.ts (new): the consent round trip on the mock, no secret rendered, the heartbeat 409 copy, the chunk size. Check: pnpm typecheck; pnpm test; pnpm build && pnpm budget (scratch outDir or delete dist/ after; paste the budget line and the l7 chunk size); nothing left running. LEAD FOLLOW-UPS FROM STAGE A (yours; add these files to your list): (a) UI/views/services/index.tsx: an unset gateway service row (reason_code gateway_none or gateway_off) links to /settings/l7, one link in the page's existing link style (U3 left it for you; services/model.ts NO_SETUP_SERVICES stays as it is); (b) UI/lib/api/mock/relay.ts: let a scenario seed the relay mock, so the hub scenario of mock/scenarios.ts (added by U3: fabric hub_role true, a gateway row carrier_ok / carrier_pending) answers GET /relay consistently (hub true, the same gateway state, a metered flag where the scenario says so) while member scenarios keep gateway_none; keep relayControl working; a test for each.` },
  D10: { model: 'opus', effort: 'xhigh', brief: `TASK D10 (row "| D10 |"): relay status, settings and consents (§6.3, §12, §13.1, §13.2 proposals; §2.4 item 2). Every other D-row has LANDED: D1 (N/relay/settings.rs: the typed view and accessors), D2 (presence; the heartbeat validator function), D3 (N/relay/gateway.rs: the badge state and GatewayDetail), D4-D9, M4/M5 (the fabric's forward counters forward_refused/forward_too_large, paired hubs in fabric settings with reachable/pending/reposts), L2 (the origins list and check_internet). Read each as it now is; you assemble, you do not re-implement. N/dashboard/api_v1/relay7.rs (T0 stub; registered by T0): GET /relay = RelayStatus exactly as the contract (session or loopback; the agent's bearer reads; no secret: token and topic_secret as {set: bool}); the SSE relay frame (the full status) on any change (a change notifier the other modules call; no timer); PATCH /relay/settings (a human; bearer 401) with partial keys per RelaySettingsPatch (the carrier keys FLAT per the contract: carrier.{kind, server, mode, token, topic_secret}; null clears a secret): validation first, then consents in the pinned round-trip: carrier_enable (subject <kind>:<server>, the §6.2 copy verbatim; a metered cap set or raised while the carrier is on re-asks with the cap in warning_text and metered in addons), gateway_bit, origin_serve (the §12 copies verbatim), each hash recorded in D1's settings, a changed text asks again; a heartbeat write: 409 no_such_transport (tcp-lan, kiss-modem, meshtastic-client), for rns 409 settings_locked with a detail naming PATCH /api/v1/bridge/settings; nothing written; the rns row shown from Layer 6's setting (owner: layer6); gateway.enabled and origin.serve on a non-hub 409 needs_hub; origin.serve without LAN sharing 409 lan_off; quota.transit is NOT a relay key (it rides GET/PATCH /fabric/settings quotas via Store::set_quota: add transit to the two quotas description strings in openapi.yaml (FabricSettings and FabricSettingsPatch), then in ${ROOT}/4-interface/ui run pnpm gen:types and node scripts/gen-sse.mjs --check and list the regenerated files); POST /relay/carrier/check and /flush (a human; bearer 401) → 202 RelayAccepted or 409 consent_required, metered_cap, licensed_board, bad_request (no carrier); they hand to D11's hooks (Wave 2): until then a registered no-op hook that records the request and answers 202 (say so). As you route each Layer 7 write, REMOVE its operationId from the UNROUTED allowlist in N/tests/dashboard_auth.rs (patchRelaySettings, offerRelayPair, pairRelayHub, unpairRelayHub, checkRelayCarrier, flushRelayCarrier, browseRegistry; the routes D6 and D2 own too, once they are registered — coordinate by reading the router) and run cargo test -p node111 --test dashboard_auth: the contract walk must then answer 401 to the agent bearer and not 401 to trusted loopback or a paired session on every one of them. Proposals kind: relay in N/dashboard/api_v1/proposals.rs: the closed vocabulary; presence.interval refused unsupported_action before any transport check; acceptance runs the same validation, consent and refusal paths as the routes (share the functions), the consent answered with accept_warning on POST /proposals/{id}/decision; a body not of the shape is 502 upstream_error. N/relay/mod.rs: start() reads D1's settings and builds only what a switch turned on (netwatch subscriptions, the mdns record, the carrier hooks); tap() hands events to presence and gateway (keep the other rows' wiring lines). N/relay/consent.rs (T0 stub): the three texts and the hash rule in one place (SHA-256 hex of the exact text), reused by D6's pair_plain. Secrets never echoed, logged or framed (grep your diff for the token field name in any log line). Never adds: a task or a file write on a GET. Tests: tests/api_v1_relay7.rs (new): every code above, the consent round trips, bearer 401 on the settings/check/flush routes, no secret in any GET or frame, the proposal paths; --lib relay. Check: CARGO_TARGET_DIR=${SP}/target/node111 from ${ROOT}/1-bootstrap: cargo test -p node111 --test api_v1_relay7 --lib relay --lib dashboard::api_v1; cargo clippy -p node111 --all-targets -- -D warnings; then in ${ROOT}/4-interface/ui: pnpm gen:types && node scripts/gen-sse.mjs --check && pnpm typecheck && pnpm test. LEAD RULINGS FROM THE U2 REVIEW (implement them; they are D33 candidates the lead records): (a) POST /relay/carrier/check and /flush carry no accept_warning in the contract, so a human who meets 409 consent_required there accepts through PATCH /relay/settings: a PATCH whose accept_warning equals the hash of the carrier_enable text owed NOW for the carrier the node holds records that consent even when the body changes no stored value; a stale or foreign hash answers 409 consent_required with the current text; test both, and check and flush answering 202 afterwards. (b) origin.targets: the node refuses, with a problem whose detail says why, a PATCH that would remove its own target triple from the list (the UI offers Remove on every entry and shows the refusal verbatim); if the contract has no fact naming the node's own target, say so under deviations. (c) a paired hub on a private address that the owner did not mark lan: say in RelayStatus what the link really runs (the UI words tls none + lan false as unknown today) and name the value under notes so the UI can word it.` },
}

// D3b also carries every item of D4c (both touch N/fabric/mod.rs: one agent, one pass).
T.D3b.brief += '\n\nROW D4c IN FULL (yours too, in this same pass):\n' + T.D4c.brief

const RESULT = {
  type: 'object',
  required: ['task', 'status', 'files', 'check', 'deviations', 'notes'],
  properties: {
    task: { type: 'string' },
    status: { type: 'string', enum: ['done', 'partial', 'blocked'] },
    files: { type: 'array', items: { type: 'string' } },
    check: { type: 'string' },
    deviations: { type: 'array', items: { type: 'string' } },
    notes: { type: 'string' },
  },
}

const REVIEW = {
  type: 'object',
  required: ['task', 'findings', 'sound', 'coverage_gaps'],
  properties: {
    task: { type: 'string' },
    findings: { type: 'array', items: { type: 'object', required: ['severity', 'verdict', 'where', 'defect', 'scenario', 'fix'], properties: {
      severity: { type: 'string', enum: ['high', 'medium', 'low'] },
      verdict: { type: 'string', enum: ['CONFIRMED', 'PLAUSIBLE'] },
      where: { type: 'string' }, defect: { type: 'string' }, scenario: { type: 'string' }, fix: { type: 'string' } } } },
    sound: { type: 'array', items: { type: 'string' } },
    coverage_gaps: { type: 'array', items: { type: 'string' } },
  },
}

// Rows whose diff gets an independent read-only adversarial review as soon as the row returns (the Wave 0 lesson: the lead's read alone
// missed three confirmed defects in M4). Doc-only rows (K1, K2) are read by the lead.
const REVIEWED = new Set(['D5', 'D4', 'D9', 'D3', 'D7', 'U1', 'U3', 'D6', 'D8', 'D2', 'A1', 'U2', 'D10', 'D4b', 'D4c', 'D3b', 'D5b', 'K4'])

const review = (r, ph) => agent(`You are a read-only adversarial reviewer in ${ROOT} (the 111network mesh OS, Layer 7 build Wave 1). Do NOT run cargo,
pnpm, node or any build; do NOT edit any file; do NOT start any process; read only (cat/sed/grep/git diff are fine). Other agents are editing
other files right now: review ONLY the files the task reports (below), as they are on disk now, and the uncommitted diff of those files
(cd ${ROOT} && git diff --histogram -- <those paths>; a new file has no diff: read it whole).

The task: row "| ${r.id} |" of ${ROOT}/7-relay/TASKS.md (read the row whole: Task, Criteria, Check, Files, the "never adds" clause); the spec
sections it names in ${ROOT}/7-relay/README.md; the contract ${ROOT}/4-interface/api/openapi.yaml (the "Layer 7" block) for any route or shape.
The author's own report (its declared deviations are NOT findings; report what they did not declare):
${JSON.stringify(r).slice(0, 7000)}

Hunt for: (1) correctness bugs reachable from a real path (trace them; cite file:line in the CURRENT file); (2) a criterion of the row not met or
met only by a test helper; (3) the "never adds" clause broken (a resident task, timer, poll, listener, file write, runtime dependency, or work
that grows with the data held: the Raspberry Pi floor); (4) security: a route reachable with the agent's bearer on a write, a secret echoed,
logged or framed, foreign bytes trusted, a size unchecked before parse, a consent round-trip that can be skipped; (5) a contract mismatch (a
shape, code or event name differs from openapi.yaml); (6) tests that do not exercise the claims. Report only what you can show with a concrete
path; CONFIRMED when traced end to end, PLAUSIBLE otherwise; list what you checked and found sound and what is not covered. Set task to "${r.id}".`,
  { label: `L7:${r.id}-review`, phase: ph, schema: REVIEW, model: 'opus', effort: 'high' })

// args.extra = { <id>: '<text>' }: a lead note appended to that row's brief (e.g. an interrupted attempt whose edits are on disk).
const note = (id) => (a.extra && a.extra[id]) ? `\n\nLEAD NOTE (read first): ${a.extra[id]}` : ''

// Each row runs, and the moment it returns its diff is reviewed (no barrier between the two); the stage ends with every row's result and review.
const run = (ids, ph) => pipeline(ids.filter(id => !ONLY || ONLY.includes(id)),
  id => agent(CTX + '\n\n' + T[id].brief + note(id), { label: `L7:${id}`, phase: ph, schema: RESULT, model: T[id].model, effort: T[id].effort })
    .then(r => r && { ...r, id }),
  r => (r && REVIEWED.has(r.id) && r.status !== 'blocked') ? review(r, ph).then(v => ({ ...r, review: v })) : r
).then(rs => rs.filter(Boolean))

// args.reviewOnly = { <id>: <the author's returned result> }: only the read-only review runs (a review that died, the author's work on disk).
if (a.reviewOnly) {
  const rs = Object.entries(a.reviewOnly).map(([id, r]) => ({ ...r, id }))
  const vs = await parallel(rs.map(r => () => review(r, 'Stage A').then(v => ({ ...r, review: v }))))
  return { reviewed: vs.filter(Boolean) }
}

phase('Stage A')
const A = await run(['D5', 'D4', 'D9', 'D3', 'D7', 'U1', 'U3', 'K1', 'K2', 'D4b', 'D4c', 'D3b', 'D5b', 'K4'], 'Stage A')
log('stage A: ' + A.map(r => `${r.id}=${r.status}${r.review ? ' (' + r.review.findings.length + ' findings)' : ''}`).join(' · '))
if (a.stopAfter === 'A') return { A }
phase('Stage B')
const B = await run(['D6', 'D8', 'D2', 'A1', 'U2'], 'Stage B')
log('stage B: ' + B.map(r => `${r.id}=${r.status}${r.review ? ' (' + r.review.findings.length + ' findings)' : ''}`).join(' · '))
if (a.stopAfter === 'B') return { A, B }
phase('Stage C')
const C = await run(['D10'], 'Stage C')
log('stage C: ' + C.map(r => `${r.id}=${r.status}${r.review ? ' (' + r.review.findings.length + ' findings)' : ''}`).join(' · '))
return { A, B, C }
