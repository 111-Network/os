// Layer 6 mock (os/60-transport/README.md §2.1, §4, §8, §9, §10, §11): three adapters, conversations with foreign messages in every
// state, a relay edge, consent 409 round-trips, optional components with sizes and the licence notice, and the transport facts. The
// routes are synchronous and throw ApiError; the adapter (index.ts) wraps them in its latency, pairing gate and structuredClone.
// Nothing here is imported by production code.
import { ApiError } from "../problem";
import type {
  BridgeComponent,
  BridgeEdge,
  BridgeEdgeCreate,
  BridgeEdgePatch,
  BridgeSettings,
  BridgeSettingsPatch,
  ConsentRequired,
  EventsFrame,
  FanoutPreview,
  FanoutRequest,
  FanoutResult,
  FanoutTarget,
  ForeignConversation,
  ForeignMessage,
  ForeignSendAccepted,
  ForeignSendRequest,
  ForeignThread,
  Network,
  NetworkId,
  Problem,
  ProblemCode,
  Proposal,
  SendGroup,
  SendItem,
  SendPlan,
  TargetRefusal,
  TransportJoin,
  TransportPosturePatch,
  TransportPostureResult,
  TransportStatus,
} from "../types";
import { sha256Hex } from "./sha256";

// --- Copy (the daemon sends the adapter manifests' strings; the UI holds none of it) ------------------------------------

const MESH_COPY: Network["copy"] = {
  banner: "Messages here are Meshtastic texts received by your board's own node; they are not 111-sealed and are labelled with their Meshtastic sender.",
  send_warning:
    "111 transmits on Meshtastic only what you send or choose to bridge, as a normal text from your board's own node. Anyone on that channel can read it: the default channel's key is public.",
  relay_tag: "M",
  auth: { "channel-key": "anyone with this channel key can use any name", pki: "the board's PKI proved the sender key, not the name" },
  dm_note: "Send a Meshtastic DM from this node (readable, not 111-sealed)",
};
const LXMF_COPY: Network["copy"] = {
  banner:
    "Messages here are LXMF texts carried by Reticulum through this node's own helper. They are not 111-sealed and are labelled with their LXMF sender address. While the helper is off, this node receives no LXMF.",
  send_warning:
    "111 sends on LXMF only what you send or choose to bridge, as a message from this node's bridge identity (its own LXMF address). The recipient can read it; delivery depends on the path, and LXMF confirms it only when the recipient's node does.",
  relay_tag: "LX",
  auth: {
    none: "nothing vouches for the sender's name; the library did not validate a signature (the reason is shown)",
    signed: "the LXMF library validated the sender's signature for this address; the name is still self-declared",
  },
  dm_note: "Send an LXMF message from this node (readable, not 111-sealed)",
};
const DC_COPY: Network["copy"] = {
  banner:
    "This hub decrypts everything its Delta Chat bridge account receives, including messages from contacts you do not bridge. The plaintext exists on this hub: the hub terminates Delta Chat's end-to-end encryption. Bridging is off for every contact until you turn it on for that one.",
  send_warning: "111 sends from this hub's own Delta Chat bridge account, flagged as a bot. What you relay onto another network is readable there.",
  relay_tag: "DC",
  auth: { none: "the app's end-to-end encryption ends on this hub; 111 vouches for nothing about the sender's name" },
  dm_note: "Open Delta Chat on your phone or desktop: that is the native app; Messages shows the same conversations as text",
};

export const MESH_AIRTIME_RULE = "the time-on-air formula for the board's modem preset (255 B on LongFast is about 2.1 s)";

/** The sentence the daemon sends with each `reason_code`; the UI keeps its own copy of the known ones. */
export const FOREIGN_REASONS: Record<string, string> = {
  region_unset: "The region is not set, so the board transmits nothing yet.",
  no_channel: "The board has no such channel.",
  too_large: "The text is larger than one packet carries.",
  not_authorized: "The board refused the send.",
  pki_failed: "The board could not encrypt this direct message.",
  got_nak: "The board answered with a refusal.",
  adapter_down: "The network is not reachable right now.",
  no_route: "Transmitted, but no route was found. Not confirmed.",
  timeout: "Transmitted, but no confirmation came back in time.",
  max_retransmit: "Transmitted, retried, and still not confirmed.",
  no_response: "Transmitted, and the other side did not answer.",
  duty_cycle: "The region's duty-cycle cap is reached; it goes out later.",
  rate_limited: "The radio asked for a pause between texts; it goes out later.",
  key_unknown: "The board does not have their key yet; the message goes out once it hears their node.",
  peer_unannounced: "No announce from them yet, so there is no path.",
  uplink_off: "Sending on this channel is off: it is published to the internet by your board.",
  ham_unconfirmed: "Your board is amateur-licensed and you have not confirmed readable sends.",
  consent_paused: "Something you confirmed has changed, so the relay paused.",
  cancelled: "You cancelled this before it left.",
  oversize: "Too long for one packet.",
  handoff_pending: "It may have been sent before this node stopped. Resend if you want it sent again.",
};

const EVIDENCE: Record<string, Record<string, string>> = {
  meshtastic: { queued: "waiting for the radio", sent: "accepted by the radio", relayed: "a neighbour rebroadcast it", delivered: "the recipient confirmed", failed: "refused before air" },
  lxmf: { queued: "no path yet", sent: "handed to the network", relayed: "a propagation node holds it", delivered: "the recipient's node confirmed", failed: "refused" },
  "app:deltachat": { queued: "pending", sent: "handed to the relay", relayed: "the relay accepted it", delivered: "the recipient read it", failed: "refused" },
};

// --- Data ----------------------------------------------------------------------------------------------------------------

export const MESH_ACCOUNT = "Bench radio (!a1b2c3d4)";
export const LXMF_ACCOUNT = "9f3c0a77e12b84d65a0cf19e22b7d340";
export const KNOWN_REGIONS = ["US", "EU_433", "EU_868", "CN", "JP", "ANZ", "KR", "TW", "RU", "IN", "NZ_865", "TH", "UA_433", "UA_868", "MY_433", "MY_919", "SG_923", "LORA_24"];
export const LXMF_PEER = "0123456789abcdef0123456789abcdef";
export const LXMF_PEER_2 = "fedcba9876543210fedcba9876543210";

export interface BridgeData {
  conversations: ForeignConversation[];
  messages: ForeignMessage[];
  edges: BridgeEdge[];
  settings: BridgeSettings;
  components: BridgeComponent[];
  transport: TransportStatus;
  groups: SendGroup[];
  /** The board reports an amateur-licensed user (read live). */
  licensed: boolean;
  /** The board reports the channel uplink bit per conversation: `ForeignConversation.uplink`. */
  dropped: Record<NetworkId, number>;
  /** Whether the bridge for this network is switched on (compiled-in adapters; optional ones follow their component). */
  nextSeq: number;
  nextFm: number;
  nextFc: number;
  nextEdge: number;
  nextGroup: number;
}

function component(id: string, adapter: NetworkId, over: Partial<BridgeComponent>, notice: string, spdx: string, readAt: string): BridgeComponent {
  return {
    id,
    adapter,
    state: "cached",
    cached_bytes: 9_000_000,
    installed_bytes: null,
    rss_bytes: null,
    license: { spdx, read_at: readAt, notice },
    notice_hash: sha256Hex(notice),
    acknowledged: false,
    reason_code: null,
    reason: null,
    ...over,
  };
}

const LXMF_NOTICE = "Reticulum and LXMF (the Reticulum License, a non-OSI licence) are downloaded from PyPI, hash-locked, into this node's cache: 9.0 MB. Read the licence at https://github.com/markqvist/Reticulum/blob/master/LICENSE. You install it; 111 is not the distributor.";
const DC_NOTICE = "deltachat-rpc-server (MPL-2.0) and the Madmail relay (AGPL-3.0) are separate programs: 31.4 MB cached. Read the licences at https://github.com/chatmail/core and https://github.com/themadorg/madmail. You install them; 111 is not the distributor.";

export function buildComponents(): BridgeComponent[] {
  return [
    component("lxmf", "lxmf", { cached_bytes: 9_000_000, rss_bytes: 38_000_000 }, LXMF_NOTICE, "LicenseRef-Reticulum", "https://github.com/markqvist/Reticulum/blob/master/LICENSE"),
    component("deltachat", "app:deltachat", { cached_bytes: 31_400_000, rss_bytes: 60_000_000 }, DC_NOTICE, "MPL-2.0 AND AGPL-3.0-only", "https://github.com/chatmail/core"),
  ];
}

export function buildTransport(region: string | null): TransportStatus {
  return {
    schema: 1,
    interfaces: [
      { name: "Default Interface", type: "AutoInterface", up: true, rx: 18_420, tx: 9_113, peers: 2 },
      { name: "RNode 1", type: "RNodeInterface", up: false, rx: 0, tx: 0, peers: null },
    ],
    interfaces_state: "ok",
    posture: { share_instance: true, instance_name: "111", enable_transport: false, lan: { backbone_listen: false, port: 4242, ifac: false }, stock_autointerface: false, propagation_node: false },
    region,
    callsign: null,
    order: ["111", "lxmf", "meshtastic", "app:deltachat", "app:simplex"],
  };
}

/** A quiet node: no region set, no board facts, nothing enabled, no conversations. The default of every scenario. */
export function buildBridgeQuiet(): BridgeData {
  return {
    conversations: [],
    messages: [],
    edges: [],
    settings: {
      schema: 1,
      order: ["111", "lxmf", "meshtastic", "app:deltachat", "app:simplex"],
      adapters: { meshtastic: true },
      recipient_lists: [],
      channels: [],
      ham: [],
      announce: { name: null, interval_min: 60 },
    },
    components: buildComponents(),
    transport: buildTransport(null),
    groups: [],
    licensed: false,
    dropped: {},
    nextSeq: 1,
    nextFm: 1,
    nextFc: 1,
    nextEdge: 1,
    nextGroup: 1,
  };
}

const MIN = 60_000;
const iso = (ms: number): string => new Date(ms).toISOString();

interface Seed {
  conv: string;
  dir: "in" | "own";
  text: string;
  state?: ForeignMessage["state"];
  reason?: string | null;
  sender?: { native: string; name: string };
  auth?: ForeignMessage["auth"];
  ago: number;
  group?: string | null;
  part?: { k: number; n: number } | null;
  relayed_from?: ForeignMessage["relayed_from"];
  via_internet?: boolean;
  claim?: number | null;
  truncated?: number | null;
}

/** The rich node: a Meshtastic board with a region and a licensed user, two channels (one uplinked) and a DM; LXMF live with a recipient list; Delta Chat cached. */
export function buildBridgeRich(now: number): BridgeData {
  const d = buildBridgeQuiet();
  d.transport = buildTransport("EU_868");
  d.licensed = true;
  d.dropped = { meshtastic: 3 };
  d.settings.recipient_lists = [{ id: "friends", network: "lxmf", name: "Friends", members: [LXMF_PEER, LXMF_PEER_2] }];
  d.settings.adapters = { meshtastic: true };
  d.settings.ham = [{ account: MESH_ACCOUNT, licensed: true, confirmed: false }];
  d.settings.channels = [
    { conv: "fc-1", network: "meshtastic", send: true, uplink: false },
    { conv: "fc-2", network: "meshtastic", send: false, uplink: true },
  ];
  const lx = d.components.find((c) => c.id === "lxmf")!;
  lx.state = "running";
  lx.installed_bytes = 24_000_000;
  lx.acknowledged = true;
  const conv = (id: string, over: Partial<ForeignConversation>): ForeignConversation => ({
    id,
    network: "meshtastic",
    account: MESH_ACCOUNT,
    kind: "channel",
    native_id: "",
    title: "",
    last_seq: 0,
    unread: 0,
    pinned: false,
    uplink: null,
    send_enabled: null,
    ...over,
  });
  d.conversations = [
    conv("fc-1", { title: "LongFast", native_id: "ch:LongFast:7f3a9c", unread: 2, uplink: false, send_enabled: true }),
    conv("fc-2", { title: "Ridge crew", native_id: "ch:Ridge crew:1b44e0", unread: 1, pinned: true, uplink: true, send_enabled: false }),
    conv("fc-3", { title: "Mara's handheld", native_id: "dm:!0f3e9a12", kind: "dm", unread: 1 }),
    conv("fc-4", { title: "Friends", native_id: "list:friends", network: "lxmf", account: LXMF_ACCOUNT, kind: "group" }),
    conv("fc-5", { title: "Sam (Sideband)", native_id: `dm:${LXMF_PEER}`, network: "lxmf", account: LXMF_ACCOUNT, kind: "dm" }),
  ];
  d.nextFc = 6;
  const A = { native: "!a1b2c3d4", name: "Ridge repeater" };
  const HOSTILE = { native: "!5be00417", name: '<a href="https://evil.example/x">click here</a>‮FREE' };
  const MARA = { native: "!0f3e9a12", name: "Mara" };
  const seeds: Seed[] = [
    { conv: "fc-1", dir: "in", text: "Weather turning, stay off the ridge.", sender: A, auth: "channel-key", ago: 50 * MIN, claim: -3 * MIN },
    { conv: "fc-1", dir: "in", text: "<img src=x onerror=alert(1)> Free WiFi", sender: HOSTILE, auth: "channel-key", ago: 44 * MIN, via_internet: true },
    { conv: "fc-1", dir: "own", text: "Anyone near the south trailhead?", state: "delivered", group: "sg-1", ago: 40 * MIN },
    { conv: "fc-1", dir: "own", text: "Battery swap at noon.", state: "relayed", group: "sg-2", ago: 30 * MIN },
    { conv: "fc-1", dir: "own", text: "Heading down now.", state: "sent", group: "sg-3", ago: 20 * MIN },
    { conv: "fc-1", dir: "own", text: "Waiting on the radio.", state: "queued", reason: "rate_limited", group: "sg-4", ago: 10 * MIN },
    { conv: "fc-1", dir: "own", text: "Refused by the board.", state: "failed", reason: "no_channel", group: "sg-5", ago: 9 * MIN },
    { conv: "fc-1", dir: "own", text: "(1/3) Long update, part one of three.", state: "relayed", group: "sg-6", part: { k: 1, n: 3 }, ago: 8 * MIN },
    { conv: "fc-1", dir: "own", text: "(2/3) Part two goes out second.", state: "sent", group: "sg-6", part: { k: 2, n: 3 }, ago: 8 * MIN },
    { conv: "fc-1", dir: "own", text: "(3/3) Part three is still waiting.", state: "queued", group: "sg-6", part: { k: 3, n: 3 }, ago: 8 * MIN },
    { conv: "fc-1", dir: "own", text: "Sent just before the node stopped.", state: "queued", reason: "handoff_pending", group: "sg-7", ago: 7 * MIN },
    { conv: "fc-1", dir: "in", text: `${"This is a very long message. ".repeat(12)}`, sender: A, auth: "channel-key", ago: 6 * MIN, truncated: 70_000 },
    { conv: "fc-2", dir: "in", text: "Camp is set up at the second lake.", sender: MARA, auth: "channel-key", ago: 15 * MIN },
    { conv: "fc-3", dir: "in", text: "Are you at the ridge?", sender: MARA, auth: "pki", ago: 12 * MIN },
    { conv: "fc-3", dir: "own", text: "Yes, up here until four.", state: "delivered", group: "sg-8", ago: 11 * MIN },
    { conv: "fc-3", dir: "own", text: "Bring the long cable.", state: "queued", reason: "key_unknown", group: "sg-9", ago: 5 * MIN },
    { conv: "fc-4", dir: "own", text: "[Ridge repeater/M] Weather turning, stay off the ridge.", state: "delivered", relayed_from: { msg: "fm-1", edge: "be-1" }, ago: 49 * MIN },
    { conv: "fc-4", dir: "own", text: "[Mara/M] Camp is set up at the second lake.", state: "sent", relayed_from: { msg: "fm-13", edge: "be-1" }, ago: 14 * MIN },
    { conv: "fc-5", dir: "in", text: "Hi from Sideband.", sender: { native: LXMF_PEER, name: "Sam" }, auth: "none", ago: 25 * MIN, claim: -2 * MIN },
    { conv: "fc-5", dir: "own", text: "Got it, thanks.", state: "delivered", group: "sg-10", ago: 24 * MIN },
  ];
  for (const s of seeds) {
    const seq = d.nextSeq++;
    const id = `fm-${d.nextFm++}`;
    const net = d.conversations.find((c) => c.id === s.conv)!.network;
    const state = s.dir === "in" ? "delivered" : (s.state ?? "queued");
    d.messages.push({
      id,
      conv: s.conv,
      dir: s.dir,
      sender: s.dir === "in" ? (s.sender ?? A) : { native: net === "lxmf" ? LXMF_ACCOUNT : "!a1b2c3d4", name: "" },
      auth: s.dir === "in" ? (s.auth ?? "none") : "none",
      text: s.text,
      state,
      evidence: s.dir === "in" ? null : (EVIDENCE[net]?.[state] ?? null),
      reason_code: s.reason ?? null,
      reason: s.reason ? (FOREIGN_REASONS[s.reason] ?? null) : null,
      group_id: s.dir === "own" && s.group ? s.group : null,
      part: s.part ?? null,
      relayed_from: s.relayed_from ?? null,
      via_internet: s.via_internet === true,
      seq,
      at: iso(now - s.ago),
      claim_ts: s.claim != null ? iso(now - s.ago + s.claim) : null,
      truncated: s.truncated ?? null,
    });
    const c = d.conversations.find((x) => x.id === s.conv)!;
    c.last_seq = seq;
  }
  d.nextGroup = 11;
  d.edges = [
    edgeRow("be-1", "fc-1", "fc-4", "channel", "group", { consent_hash: sha256Hex(edgeText(d, "fc-1", "fc-4").text), counters: { relayed: 2, shed: 1, refused: 3 }, cursor_seq: 0, relayed_seq: 12 }),
    edgeRow("be-2", "fc-1", "fc-2", "channel", "channel", {
      paused: true,
      pause_reason: "consent_paused",
      consent_hash: sha256Hex("Everyone in LongFast will have their messages copied to Ridge crew. They see the [name/TAG] prefix only if they read Ridge crew."),
      counters: { relayed: 4, shed: 0, refused: 0 },
      relayed_seq: 11,
    }),
  ];
  d.nextEdge = 3;
  return d;
}

function edgeRow(id: string, src: string, dst: string, src_kind: BridgeEdge["src_kind"], dst_kind: BridgeEdge["dst_kind"], over: Partial<BridgeEdge>): BridgeEdge {
  return {
    id,
    src,
    dst,
    src_kind,
    dst_kind,
    enabled: true,
    enabled_by: "loopback",
    paused: false,
    cursor_seq: 0,
    relayed_seq: 0,
    honour_ok_to_mqtt: false,
    oversize: "refuse",
    consent_hash: null,
    counters: { relayed: 0, shed: 0, refused: 0 },
    pause_reason: null,
    ...over,
  };
}

/** The warning text for an edge, with the live add-ons (§8): a default-key source, an uplinked destination, a licensed destination account. */
function edgeText(d: BridgeData, src: string, dst: string): { text: string; addons: string[] } {
  const s = d.conversations.find((c) => c.id === src);
  const t = d.conversations.find((c) => c.id === dst);
  const sTitle = s?.title ?? src;
  const tTitle = t?.title ?? dst;
  const addons: string[] = [];
  let text = `Everyone in ${sTitle} will have their messages copied to ${tTitle}. They see the [name/TAG] prefix only if they read ${tTitle}.`;
  if (s && s.network === "meshtastic" && s.kind === "channel" && /LongFast/.test(s.title)) {
    addons.push("default_key_source");
    text += ` ${sTitle} uses the default key: anyone can read it; this copies public traffic.`;
  }
  if (t?.uplink) {
    addons.push("uplinked_destination");
    text += ` ${tTitle} is published to the internet by your board.`;
  }
  if (t && t.network === "meshtastic" && d.licensed) {
    addons.push("licensed_destination");
    text += ` The board for ${tTitle} is amateur-licensed: third-party traffic is the licensee's fact.`;
  }
  return { text, addons };
}

// --- The routes -----------------------------------------------------------------------------------------------------------

export interface BridgeEnv {
  now(): number;
  later(ms: number, fn: () => void): void;
  publish(frame: EventsFrame): void;
  /** Queue a sealed 111 message to an accepted contact (the Layer 5 path); returns its `msg-…` id. Throws 404 when `nodeId` is not a contact. */
  sendTo111(nodeId: string, text: string): string;
}

function problem(status: number, code: ProblemCode, title: string, detail?: string, extra: Partial<Problem> = {}): ApiError {
  const p: Problem = { type: "about:blank", title, status, code, ...extra };
  if (detail) p.detail = detail;
  return new ApiError(p);
}

const bytesOf = (s: string): number => new TextEncoder().encode(s).length;

export function createBridgeMock(d: BridgeData, env: BridgeEnv) {
  const conv = (id: string): ForeignConversation => {
    const c = d.conversations.find((x) => x.id === id);
    if (!c) throw problem(404, "not_found", "No such conversation", id);
    return c;
  };
  const row = (id: string): ForeignMessage | undefined => d.messages.find((m) => m.id === id);

  // --- networks, derived from the facts (like the daemon's registry) ---
  const component_ = (adapter: NetworkId): BridgeComponent | undefined => d.components.find((c) => c.adapter === adapter);
  const region = (): string | null => d.transport.region;

  function networkOf(id: NetworkId): Network {
    const convs = d.conversations.filter((c) => c.network === id);
    const queued = d.messages.filter((m) => m.dir === "own" && m.state === "queued" && convs.some((c) => c.id === m.conv)).length;
    const counts = {
      conversations: convs.length,
      unread: convs.filter((c) => c.kind === "dm" || c.pinned).reduce((n, c) => n + c.unread, 0),
      queued,
      edges: d.edges.filter((e) => convs.some((c) => c.id === e.src || c.id === e.dst)).length,
    };
    if (id === "meshtastic") {
      const set = region() !== null;
      const off = d.settings.adapters.meshtastic === false;
      return {
        id,
        label: "Meshtastic",
        chip: "M",
        state: off ? "disabled" : set ? "live" : "not_set",
        reason_code: off ? "adapter_off" : set ? null : "region_unset",
        reason: off ? "Meshtastic is turned off here." : set ? null : "The region is not set, so the board transmits nothing. Pick your region in Settings.",
        account: MESH_ACCOUNT,
        copy: MESH_COPY,
        caps: {
          kinds: ["channel", "dm"],
          max_text_bytes: { channel: { default: 232 }, dm: { default: 220, licensed: 232 } },
          evidence: ["queued", "sent", "relayed", "delivered", "failed"],
          airtime: { duty_cycle_permille: 100, bits_per_s: 1070, rule: MESH_AIRTIME_RULE },
          auth_levels: ["channel-key", "pki"],
          splittable: true,
          recipient_syntax: { label: "Meshtastic node id", pattern: "^![0-9a-f]{8}$", example: "!a1b2c3d4" },
        },
        gate: { region_set: set, licensed: d.licensed, ham_confirmed: d.licensed ? (d.settings.ham.find((h) => h.account === MESH_ACCOUNT)?.confirmed ?? false) : null, uplink_channels: d.conversations.filter((c) => c.uplink === true).length },
        counts,
        dropped: d.dropped.meshtastic ?? 0,
        component: null,
      };
    }
    const comp = component_(id) ?? null;
    const on = comp?.state === "running";
    if (id === "lxmf") {
      return {
        id,
        label: "Reticulum (LXMF)",
        chip: "LX",
        state: on ? "live" : "disabled",
        reason_code: on ? null : "component_unavailable",
        reason: on ? null : "The Reticulum helper is not enabled.",
        account: on ? LXMF_ACCOUNT : null,
        copy: LXMF_COPY,
        caps: {
          kinds: ["dm", "group"],
          max_text_bytes: { dm: { default: 287 }, group: { default: 287 } },
          evidence: ["queued", "sent", "relayed", "delivered"],
          airtime: null,
          auth_levels: ["none", "signed"],
          splittable: true,
          recipient_syntax: { label: "LXMF address (32 hex)", pattern: "^[0-9a-f]{32}$", example: "0123456789abcdef0123456789abcdef" },
        },
        gate: { region_set: null, licensed: null, ham_confirmed: null, uplink_channels: 0 },
        counts,
        dropped: d.dropped.lxmf ?? 0,
        component: comp,
      };
    }
    return {
      id,
      label: "Delta Chat",
      chip: "DC",
      state: on ? "live" : "disabled",
      reason_code: on ? null : "component_unavailable",
      reason: on ? null : "The Delta Chat bridge is not enabled.",
      account: on ? "bridge@hub.local" : null,
      copy: DC_COPY,
      caps: {
        kinds: ["dm", "group"],
        max_text_bytes: { dm: { default: 4000 }, group: { default: 4000 } },
        evidence: ["queued", "sent", "relayed", "delivered"],
        airtime: null,
        auth_levels: ["none"],
        splittable: false,
        recipient_syntax: null,
      },
      gate: { region_set: null, licensed: null, ham_confirmed: null, uplink_channels: 0 },
      counts,
      dropped: 0,
      component: comp,
    };
  }

  const networkIds = (): NetworkId[] => {
    const known: NetworkId[] = ["meshtastic", "lxmf", "app:deltachat"];
    const rank = (n: string) => {
      const i = d.settings.order.indexOf(n);
      return i < 0 ? 99 : i;
    };
    return known.sort((a, b) => rank(a) - rank(b));
  };
  const networks = (): Network[] => networkIds().map(networkOf);
  const network = (id: string): Network => {
    if (!networkIds().includes(id)) throw problem(404, "not_found", "No such network", id);
    return networkOf(id);
  };
  const publishBridge = (parts: { networks?: boolean; edges?: boolean }) =>
    env.publish({ event: "bridge", data: { ...(parts.networks ? { networks: structuredClone(networks()) } : {}), ...(parts.edges ? { edges: structuredClone(d.edges) } : {}) } });
  const highSeq = (id: string): number => d.messages.reduce((n, m) => (m.conv === id && m.seq > n ? m.seq : n), 0);
  const frame = (m: ForeignMessage, change: "new" | "state" | "conversation") => {
    const c = conv(m.conv);
    env.publish({ event: "network_message", data: { network: c.network, conv: c.id, ...(change === "conversation" ? {} : { id: m.id }), change, seq: highSeq(c.id) } });
  };

  // --- consent texts (read live) ---
  const hamText = (account: string): ConsentRequired => {
    const text = `Your board ${account} reports an amateur licence. Readable texts sent or relayed through it go out on amateur frequencies under your call sign; the content and third-party-traffic rules are yours to keep. Sealed 111 frames never go this way. Confirm only if you are the licensee.`;
    return { action: "ham_confirm", subject: account, warning_text: text, hash: sha256Hex(text), addons: ["licensed_account"] };
  };
  const uplinkText = (c: ForeignConversation): ConsentRequired => {
    const text = `${c.title}: this channel is published to the internet by your board. Everything sent on it, relayed copies included, is readable on the MQTT server.`;
    return { action: "uplink_send", subject: c.id, warning_text: text, hash: sha256Hex(text), addons: ["uplinked_channel"] };
  };
  const consentFail = (c: ConsentRequired): ApiError => problem(409, "consent_required", "Confirmation needed", "Read the text and confirm it to continue.", { consent: c });

  const hamConfirmed = (): boolean => d.settings.ham.find((h) => h.account === MESH_ACCOUNT)?.confirmed === true;

  // --- plans ---
  function planFor(c: ForeignConversation, text: string, mode: "refuse" | "split"): SendPlan {
    const net = networkOf(c.network);
    const caps = net.caps.max_text_bytes[c.kind] ?? {};
    const cap = (c.kind === "dm" && d.licensed ? caps.licensed : undefined) ?? caps.default ?? null;
    const raw = bytesOf(text);
    const over = cap !== null && raw > cap;
    const parts = over && mode === "split" && net.caps.splittable && cap !== null ? Math.ceil(raw / (cap - 8)) : 1;
    const warnings: SendPlan["warnings"] = [];
    let refusal: SendPlan["refusal"] = null;
    if (over && (mode === "refuse" || !net.caps.splittable)) refusal = { code: "oversize", detail: `${raw} B > ${cap} B per packet`, consent: null };
    if (c.network === "meshtastic") {
      if (!refusal && d.licensed && !hamConfirmed()) {
        const consent = hamText(MESH_ACCOUNT);
        refusal = { code: "ham_unconfirmed", detail: "Your board is amateur-licensed: confirm readable sends once first.", consent };
      }
      if (!refusal && c.kind === "channel" && c.send_enabled === false) {
        const consent = uplinkText(c);
        refusal = { code: "uplink_off", detail: `${c.title} is published to the internet by your board; sending there is off until you confirm.`, consent };
      }
      if (region() === null) warnings.push({ code: "region_unset", text: "The region is not set, so the board transmits nothing yet; the text waits." });
      if (c.uplink && c.send_enabled) warnings.push({ code: "uplink_channel", text: "This channel is published to the internet by your board." });
      if (parts > 1) warnings.push({ code: "parts_out_of_order", text: "Parts can arrive out of order." });
    }
    if (net.state === "error") warnings.push({ code: "adapter_down", text: "The network is down; the text waits on this node." });
    const total = raw + (parts > 1 ? 6 * parts : 0);
    const airtime: SendPlan["airtime_estimate"] =
      c.network === "meshtastic"
        ? { millis: Math.max(200, Math.round((2100 * Math.min(total, cap ?? total)) / 255)) * parts, label: `about ${((Math.max(200, Math.round((2100 * Math.min(total, cap ?? total)) / 255)) * parts) / 1000).toFixed(1)} s (estimate)` }
        : c.network === "lxmf"
          ? { millis: null, label: "depends on the path" }
          : null;
    return { bytes: total, cap, parts, size_mode: mode, airtime_estimate: airtime, refusal, warnings };
  }

  function partsOf(text: string, parts: number): string[] {
    if (parts <= 1) return [text];
    const chars = Array.from(text);
    const per = Math.ceil(chars.length / parts);
    return Array.from({ length: parts }, (_, i) => `(${i + 1}/${parts}) ${chars.slice(i * per, (i + 1) * per).join("")}`);
  }

  // --- delivery (a stand-in for the adapter: monotone, evidence strings from the table) ---
  const setState = (m: ForeignMessage, state: ForeignMessage["state"], reason: string | null) => {
    m.state = state;
    m.evidence = EVIDENCE[conv(m.conv).network]?.[state] ?? null;
    m.reason_code = reason;
    m.reason = reason ? (FOREIGN_REASONS[reason] ?? null) : null;
    frame(m, "state");
  };
  const chain = new Set<string>();
  function dispatch(m: ForeignMessage): void {
    if (m.dir !== "own" || chain.has(m.id) || m.reason_code === "cancelled") return;
    const c = conv(m.conv);
    const net = networkOf(c.network);
    if (net.state === "disabled" || net.state === "error") return void setState(m, "queued", "adapter_down");
    if (c.network === "meshtastic" && region() === null) return void setState(m, "queued", "region_unset");
    const steps: [number, ForeignMessage["state"]][] =
      c.network === "meshtastic" ? (c.kind === "dm" ? [[400, "sent"], [900, "relayed"], [1500, "delivered"]] : [[400, "sent"], [900, "relayed"]]) : [[400, "sent"], [1200, "delivered"]];
    chain.add(m.id);
    if (m.state === "queued" && m.reason_code) setState(m, "queued", null);
    const next = (i: number) => {
      const step = steps[i];
      if (!step) return void chain.delete(m.id);
      env.later(step[0], () => {
        if (!d.messages.includes(m) || m.reason_code === "cancelled") return void chain.delete(m.id);
        setState(m, step[1], null);
        next(i + 1);
      });
    };
    next(0);
  }

  function queueRows(c: ForeignConversation, text: string, plan: SendPlan, groupId: string): ForeignMessage[] {
    const texts = partsOf(text, plan.parts);
    return texts.map((t, i) => {
      const m: ForeignMessage = {
        id: `fm-${d.nextFm++}`,
        conv: c.id,
        dir: "own",
        sender: { native: c.network === "lxmf" ? LXMF_ACCOUNT : "!a1b2c3d4", name: "" },
        auth: "none",
        text: t,
        state: "queued",
        evidence: EVIDENCE[c.network]?.queued ?? null,
        reason_code: null,
        reason: null,
        group_id: groupId,
        part: plan.parts > 1 ? { k: i + 1, n: plan.parts } : null,
        relayed_from: null,
        via_internet: false,
        seq: d.nextSeq++,
        at: iso(env.now()),
        claim_ts: null,
        truncated: null,
      };
      d.messages.push(m);
      c.last_seq = m.seq;
      frame(m, "new");
      return m;
    });
  }

  const sendMode = (m: unknown): "refuse" | "split" => (m === "split" ? "split" : "refuse");

  // --- the adapter surface (synchronous; index.ts wraps each in `rest`) ---
  const api = {
    networks: {
      list: (): Network[] => networks(),
      conversations: (net: string): ForeignConversation[] => {
        network(net);
        return [...d.conversations].filter((c) => c.network === net).sort((a, b) => b.last_seq - a.last_seq);
      },
      openDm: (net: string, nativeId: string): ForeignConversation => {
        const n = network(net);
        if (n.state === "disabled") throw problem(409, "component_unavailable", "That network is not enabled", n.reason ?? undefined);
        const syntax = n.caps.recipient_syntax;
        if (!syntax || !new RegExp(syntax.pattern).test(nativeId)) throw problem(400, "bad_request", "That is not an address on this network", syntax ? `Expected a ${syntax.label}, for example ${syntax.example}.` : undefined);
        const native = `dm:${nativeId}`;
        const have = d.conversations.find((c) => c.network === net && c.kind === "dm" && c.native_id === native);
        if (have) return have;
        const c: ForeignConversation = { id: `fc-${d.nextFc++}`, network: net, account: n.account ?? "", kind: "dm", native_id: native, title: nativeId, last_seq: 0, unread: 0, pinned: false, uplink: null, send_enabled: null };
        d.conversations.push(c);
        env.publish({ event: "network_message", data: { network: net, conv: c.id, change: "conversation", seq: 0 } });
        return c;
      },
      conversation: (net: string, id: string, q: { sinceSeq?: number; limit?: number } = {}): ForeignThread => {
        network(net);
        const c = conv(id);
        if (c.network !== net) throw problem(404, "not_found", "No such conversation", id);
        const limit = q.limit ?? 100;
        if (!Number.isInteger(limit) || limit < 1 || limit > 500) throw problem(400, "bad_request", "Invalid limit", undefined, { errors: [{ pointer: "/limit", message: "must be between 1 and 500" }] });
        const all = d.messages.filter((m) => m.conv === id).sort((a, b) => a.seq - b.seq);
        const page = q.sinceSeq !== undefined ? all.filter((m) => m.seq > q.sinceSeq!) : all;
        const messages = q.sinceSeq !== undefined ? page.slice(0, limit) : page.slice(-limit);
        // Reading a conversation marks it read (the daemon does the same on the foreign side's behalf: nothing is acknowledged to anyone).
        if (c.unread > 0 && q.sinceSeq === undefined) c.unread = 0;
        return { schema: 1, conversation: c, messages, more: q.sinceSeq !== undefined ? page.length > limit : all.length > limit };
      },
      send: (net: string, id: string, body: ForeignSendRequest): ForeignSendAccepted => {
        const n = network(net);
        const c = conv(id);
        if (c.network !== net) throw problem(404, "not_found", "No such conversation", id);
        if (typeof body?.text !== "string" || body.text === "") throw problem(400, "bad_request", "Invalid message", undefined, { errors: [{ pointer: "/text", message: "must not be empty" }] });
        if (n.state === "disabled") throw problem(409, "component_unavailable", "That network is not enabled", n.reason ?? undefined);
        const plan = planFor(c, body.text, sendMode(body.size_mode));
        if (plan.refusal?.code === "oversize") throw problem(409, "oversize", "Too long for one packet", plan.refusal.detail);
        if (plan.refusal?.consent) throw consentFail(plan.refusal.consent);
        const group = `sg-${d.nextGroup++}`;
        const rows = queueRows(c, body.text, plan, group);
        for (const m of rows) dispatch(m);
        publishBridge({ networks: true });
        return { schema: 1, id: rows[0]!.id, ...(rows.length > 1 ? { ids: rows.map((r) => r.id) } : {}), plan };
      },
      fanoutPreview: (body: FanoutRequest): FanoutPreview => {
        const items = targetsOf(body).map((t) => ({ target: t.target, plan: t.plan }));
        return { schema: 1, text_bytes: bytesOf(body.text), ok: items.every((i) => i.plan.refusal === null), items };
      },
      fanout: (body: FanoutRequest): FanoutResult => {
        const planned = targetsOf(body);
        const refused = planned.filter((t) => t.plan.refusal !== null);
        if (refused.length > 0) {
          const targets: TargetRefusal[] = refused.map((t) => ({ target: t.target, code: t.plan.refusal!.code === "oversize" ? ("oversize" as const) : ("consent_required" as const), detail: t.plan.refusal!.detail, consent: t.plan.refusal!.consent }));
          const gate = targets.some((t) => t.consent);
          throw problem(409, gate ? "consent_required" : "oversize", gate ? "Confirmation needed" : "Too long for one packet", `${refused.length} target${refused.length === 1 ? "" : "s"} refused; nothing was queued.`, { targets });
        }
        const group = `sg-${d.nextGroup++}`;
        const items: SendItem[] = planned.map((t) => {
          if ("node_id" in t.target) return { target: t.target, id: env.sendTo111(t.target.node_id, body.text), plan: t.plan };
          const c = conv(t.target.conv);
          const rows = queueRows(c, body.text, t.plan, group);
          for (const m of rows) dispatch(m);
          return { target: t.target, id: rows[0]!.id, ...(rows.length > 1 ? { ids: rows.map((r) => r.id) } : {}), plan: t.plan };
        });
        d.groups.push({ id: group, text: body.text, items });
        publishBridge({ networks: true });
        return { schema: 1, group_id: group, items };
      },
    },

    bridge: {
      edges: (): BridgeEdge[] => d.edges,
      createEdge: (body: BridgeEdgeCreate): BridgeEdge => {
        const src = conv(body.src);
        const dst = conv(body.dst);
        if (src.id === dst.id) throw problem(400, "bad_request", "A relay needs two different conversations");
        for (const c of [src, dst]) if (c.kind === "dm") throw problem(400, "bad_request", "A direct message cannot be relayed", "A relay joins channels or groups only.");
        if (src.kind === "group") throw problem(400, "bad_request", "A recipient list can only receive", "There is no Reticulum source conversation to read.");
        for (const c of [src, dst]) {
          const n = networkOf(c.network);
          if (n.state === "disabled") throw problem(409, "component_unavailable", `${n.label} is not enabled`, n.reason ?? undefined);
        }
        if (d.edges.some((e) => e.src === src.id && e.dst === dst.id)) throw problem(409, "bad_request", "Already a relay", "These two are already joined.");
        const warn = edgeText(d, src.id, dst.id);
        const hash = sha256Hex(warn.text);
        if (body.accept_warning !== hash) throw consentFail({ action: "edge_enable", subject: `${src.id}>${dst.id}`, warning_text: warn.text, hash, addons: warn.addons });
        const e = edgeRow(`be-${d.nextEdge++}`, src.id, dst.id, src.kind as "channel" | "group", dst.kind as "channel" | "group", {
          cursor_seq: d.nextSeq - 1,
          relayed_seq: d.nextSeq - 1,
          honour_ok_to_mqtt: body.honour_ok_to_mqtt === true,
          oversize: body.oversize === "split" ? "split" : "refuse",
          consent_hash: hash,
        });
        d.edges.push(e);
        publishBridge({ edges: true, networks: true });
        return e;
      },
      edge: (id: string): BridgeEdge => {
        const e = d.edges.find((x) => x.id === id);
        if (!e) throw problem(404, "not_found", "No such relay", id);
        return e;
      },
      patchEdge: (id: string, patch: BridgeEdgePatch): BridgeEdge => {
        const e = api.bridge.edge(id);
        if (patch.paused === false && e.paused) {
          if (e.pause_reason === "consent_paused") {
            const warn = edgeText(d, e.src, e.dst);
            const hash = sha256Hex(warn.text);
            if (patch.accept_warning !== hash) throw consentFail({ action: "edge_resume", subject: e.id, warning_text: warn.text, hash, addons: warn.addons });
            e.consent_hash = hash;
          }
          e.paused = false;
          e.pause_reason = null;
        } else if (patch.paused === true) {
          e.paused = true;
          e.pause_reason = "user";
        }
        if (typeof patch.honour_ok_to_mqtt === "boolean") e.honour_ok_to_mqtt = patch.honour_ok_to_mqtt;
        if (patch.oversize === "refuse" || patch.oversize === "split") e.oversize = patch.oversize;
        publishBridge({ edges: true });
        return e;
      },
      deleteEdge: (id: string): void => {
        const e = api.bridge.edge(id);
        d.edges = d.edges.filter((x) => x !== e);
        publishBridge({ edges: true, networks: true });
      },
      settings: (): BridgeSettings => settingsNow(),
      patchSettings: (patch: BridgeSettingsPatch): BridgeSettings => {
        // The first change that lacks a current hash decides the answer; nothing is applied before every change is cleared.
        for (const h of patch.ham ?? []) {
          if (h.confirmed) {
            const c = hamText(h.account);
            if (h.accept_warning !== c.hash) throw consentFail(c);
          }
        }
        for (const ch of patch.channels ?? []) {
          const c = conv(ch.conv);
          if (ch.send && c.uplink) {
            const k = uplinkText(c);
            if (ch.accept_warning !== k.hash) throw consentFail(k);
          }
        }
        if (patch.order) d.settings.order = patch.order;
        if (patch.adapters) {
          for (const [k, v] of Object.entries(patch.adapters)) {
            if (component_(k)) throw problem(409, "component_unavailable", "Use the component switch", "An optional adapter is turned on through its component.");
            d.settings.adapters[k] = v;
          }
        }
        if (patch.recipient_lists) {
          for (const l of patch.recipient_lists) {
            const syntax = networkOf(l.network).caps.recipient_syntax;
            for (const m of l.members) if (!syntax || !new RegExp(syntax.pattern).test(m)) throw problem(400, "bad_request", "That is not an address on this network", `${m} does not match ${syntax?.label ?? "any address"}.`);
          }
          d.settings.recipient_lists = patch.recipient_lists;
          syncLists();
        }
        for (const h of patch.ham ?? []) {
          const row = d.settings.ham.find((x) => x.account === h.account);
          if (row) row.confirmed = h.confirmed;
          else d.settings.ham.push({ account: h.account, licensed: d.licensed, confirmed: h.confirmed });
        }
        for (const ch of patch.channels ?? []) {
          const c = conv(ch.conv);
          c.send_enabled = ch.send;
          const row = d.settings.channels.find((x) => x.conv === ch.conv);
          if (row) row.send = ch.send;
        }
        if (patch.announce) d.settings.announce = { ...d.settings.announce, ...patch.announce } as BridgeSettings["announce"];
        publishBridge({ networks: true });
        return settingsNow();
      },
      components: (): BridgeComponent[] => d.components,
      enableComponent: (id: string, hash?: string): BridgeComponent => {
        const c = comp(id);
        if (c.state === "running") return c;
        if (c.state === "declared") throw problem(409, "component_unavailable", "Nothing is cached to install", "The download has to be placed in the cache first. Nothing was installed.");
        if (hash !== c.notice_hash) throw consentFail({ action: "component_enable", subject: c.id, warning_text: c.license.notice, hash: c.notice_hash, addons: [] });
        c.acknowledged = true;
        c.state = "running";
        c.installed_bytes = Math.round((c.cached_bytes ?? 9_000_000) * 2.6);
        if (c.adapter === "lxmf" && !d.conversations.some((x) => x.network === "lxmf")) seedLxmf();
        publishBridge({ networks: true });
        return c;
      },
      disableComponent: (id: string): BridgeComponent => {
        const c = comp(id);
        if (c.state === "running") c.state = "disabled";
        publishBridge({ networks: true });
        return c;
      },
      removeComponentFiles: (id: string, cache?: boolean): BridgeComponent => {
        const c = comp(id);
        if (c.state === "running") throw problem(409, "bad_request", "It is running", "Disable it first; files are never removed from under a running component.");
        c.installed_bytes = null;
        if (cache === true) c.cached_bytes = null;
        c.state = c.cached_bytes !== null ? "cached" : "declared";
        if (cache === true) c.reason_code = "payload_missing";
        publishBridge({ networks: true });
        return c;
      },
    },

    transport: {
      status: (): TransportStatus => d.transport,
      patchPosture: (patch: TransportPosturePatch): TransportPostureResult => {
        if (patch.region !== undefined && !KNOWN_REGIONS.includes(patch.region)) throw problem(400, "bad_request", "Unknown region", `${patch.region} is not in the region table.`);
        const t = d.transport;
        const before = structuredClone(t);
        if (patch.region !== undefined) t.region = patch.region;
        if ("callsign" in patch) t.callsign = patch.callsign ? patch.callsign : null;
        if (patch.order) {
          t.order = patch.order;
          d.settings.order = patch.order;
        }
        const p = patch.posture;
        if (p) {
          if (typeof p.share_instance === "boolean") t.posture.share_instance = p.share_instance;
          if (typeof p.instance_name === "string") t.posture.instance_name = p.instance_name;
          if (typeof p.enable_transport === "boolean") t.posture.enable_transport = p.enable_transport;
          if (p.lan) t.posture.lan = { ...t.posture.lan, ...p.lan };
          if (typeof p.stock_autointerface === "boolean") t.posture.stock_autointerface = p.stock_autointerface;
          if (typeof p.propagation_node === "boolean") t.posture.propagation_node = p.propagation_node;
        }
        // The probe: a propagation role does not show up in the mock within 30 s, so the change goes back (the revert path).
        if (p?.propagation_node === true) {
          d.transport = before;
          return { schema: 1, applied: false, reverted: true, reason: "The propagation role did not show up within 30 s, so the previous setting is live.", status: d.transport };
        }
        publishBridge({ networks: true });
        return { schema: 1, applied: true, reverted: false, reason: null, status: t };
      },
      join: (): TransportJoin => {
        const lan = d.transport.posture.lan;
        if (!lan.backbone_listen) throw problem(409, "bad_request", "LAN sharing is off", "There is no listener to join. Turn on the LAN listener first.");
        return { schema: 1, address: "192.168.1.42", port: lan.port, ...(lan.ifac ? { ifac_name: "111-hub", ifac_passphrase: "correct-horse-battery" } : {}) };
      },
    },

    /** `POST /messages/{id}/retry` on an `fm-…` id. */
    retry: (id: string): void => {
      const m = row(id);
      if (!m) throw problem(404, "not_found", "No such message", id);
      if (m.dir !== "own" || (m.state !== "queued" && m.state !== "sent" && m.state !== "failed")) throw problem(409, "bad_request", "Not retryable", "Only an own message that is waiting, sent or refused before air can be retried.");
      if (m.state === "failed") setState(m, "queued", null);
      else if (m.reason_code === "handoff_pending") setState(m, "queued", null);
      dispatch(m);
    },
    /** `POST /messages/{id}/cancel` on an `fm-…` id: drops a queued row with nothing sent. */
    cancel: (id: string): void => {
      const m = row(id);
      if (!m) throw problem(404, "not_found", "No such message", id);
      if (m.dir !== "own" || m.state !== "queued") throw problem(409, "bad_request", "Not cancellable", "Only a message that is still queued, with nothing sent, can be cancelled.");
      d.messages = d.messages.filter((x) => x !== m);
      frame(m, "conversation");
    },
    isForeignId: (id: string): boolean => id.startsWith("fm-"),

    /** A text arrives from the air (a script step). */
    receive: (convId: string, sender: { native: string; name: string }, text: string, o: { via_internet?: boolean } = {}): void => {
      const c = conv(convId);
      const m: ForeignMessage = {
        id: `fm-${d.nextFm++}`,
        conv: c.id,
        dir: "in",
        sender,
        auth: c.network === "meshtastic" ? (c.kind === "dm" ? "pki" : "channel-key") : "none",
        text,
        state: "delivered",
        evidence: null,
        reason_code: null,
        reason: null,
        group_id: null,
        part: null,
        relayed_from: null,
        via_internet: o.via_internet === true,
        seq: d.nextSeq++,
        at: iso(env.now()),
        claim_ts: null,
        truncated: null,
      };
      d.messages.push(m);
      c.last_seq = m.seq;
      c.unread++;
      frame(m, "new");
    },

    /** Accepting a bridge proposal: the same validation and consent round-trip as the direct route. */
    decideProposal: (p: Proposal, hash?: string): void => {
      if (p.kind === "bridge_edge") {
        const from = /^from:\s*(\S+)/m.exec(p.body)?.[1];
        const to = /^to:\s*(\S+)/m.exec(p.body)?.[1];
        if (!from || !to) throw problem(502, "upstream_error", "The proposal body is not in the expected shape");
        api.bridge.createEdge({ src: from, dst: to, honour_ok_to_mqtt: false, oversize: "refuse", ...(hash ? { accept_warning: hash } : {}) });
      } else if (p.kind === "bridge_message") {
        const [head, ...rest] = p.body.split(/\n\s*\n/);
        const ids = (head ?? "").split("\n").map((l) => /^to:\s*(\S+)/.exec(l)?.[1]).filter((x): x is string => !!x);
        const text = rest.join("\n\n");
        if (ids.length === 0 || text === "") throw problem(502, "upstream_error", "The proposal body is not in the expected shape");
        const targets: FanoutTarget[] = ids.map((cid) => ({ network: conv(cid).network, conv: cid }));
        // One gate at a time: the text of the first refused gate is asked about; the hash of that very text confirms it (the node records
        // the confirmation), and the next gate, if any, asks in turn with its own text.
        for (let guard = 0; guard < 8; guard++) {
          try {
            api.networks.fanout({ text, targets });
            return;
          } catch (e) {
            const first = e instanceof ApiError ? (e.problem.targets ?? []).find((t) => t.consent) : undefined;
            if (!first?.consent) throw e;
            if (first.consent.hash !== hash) throw consentFail(first.consent);
            api.bridge.patchSettings(first.consent.action === "ham_confirm" ? { ham: [{ account: first.consent.subject, confirmed: true, accept_warning: hash }] } : { channels: [{ conv: first.consent.subject, send: true, accept_warning: hash }] });
          }
        }
        throw problem(409, "bad_request", "The send could not be cleared");
      }
    },
  };

  function targetsOf(body: FanoutRequest): { target: FanoutTarget; plan: SendPlan }[] {
    if (typeof body?.text !== "string" || body.text === "") throw problem(400, "bad_request", "Invalid fan-out", undefined, { errors: [{ pointer: "/text", message: "must not be empty" }] });
    if (!Array.isArray(body.targets) || body.targets.length === 0) throw problem(400, "bad_request", "Invalid fan-out", undefined, { errors: [{ pointer: "/targets", message: "needs at least one target" }] });
    return body.targets.map((t) => {
      if ("node_id" in t) {
        return { target: t, plan: { bytes: bytesOf(body.text), cap: null, parts: 1, size_mode: "refuse" as const, airtime_estimate: null, refusal: null, warnings: [] } };
      }
      const c = conv(t.conv);
      return { target: t, plan: planFor(c, body.text, sendMode(t.size_mode ?? body.size_mode)) };
    });
  }

  function comp(id: string): BridgeComponent {
    const c = d.components.find((x) => x.id === id);
    if (!c) throw problem(404, "not_found", "No such component", id);
    return c;
  }

  function settingsNow(): BridgeSettings {
    return {
      ...d.settings,
      channels: d.conversations.filter((c) => c.kind === "channel" && c.network === "meshtastic").map((c) => ({ conv: c.id, network: c.network, send: c.send_enabled === true, uplink: c.uplink ?? null })),
      ham: d.licensed ? [{ account: MESH_ACCOUNT, licensed: true, confirmed: hamConfirmed() }] : d.settings.ham,
    };
  }

  /** A recipient list is a destination-only `group` conversation of its network (§6). */
  function syncLists(): void {
    for (const l of d.settings.recipient_lists) {
      const native = `list:${l.id}`;
      let c = d.conversations.find((x) => x.network === l.network && x.native_id === native);
      if (!c) {
        c = { id: `fc-${d.nextFc++}`, network: l.network, account: LXMF_ACCOUNT, kind: "group", native_id: native, title: l.name, last_seq: 0, unread: 0, pinned: false, uplink: null, send_enabled: null };
        d.conversations.push(c);
      } else c.title = l.name;
    }
  }

  function seedLxmf(): void {
    syncLists();
  }

  return { ...api, networkOf, networkList: networks };
}

export type BridgeMock = ReturnType<typeof createBridgeMock>;
