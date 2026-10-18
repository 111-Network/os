// Layer 5 mock data (os/50-messaging/README.md): contacts, conversations in every delivery state and reason, requests,
// the fabric snapshot, Meshtastic presence and the hosted panel, all shaped like api/openapi.yaml. The adapter
// (index.ts) holds the live copy; scenarios bend it. Nothing here is imported by production code.
import { encodeNodeBech32 } from "../contacts";
import type { Card, Contact, FabricStatus, FabricStore, HostedPanel, Message, NearbySource, NodeId, Thread } from "../types";
import { cardFor } from "./qr";

// --- Peers -------------------------------------------------------------------------------------------------------

export const ADA = "3a7f09c1d5e24b86a0f17c93e8d52b4670a9c1e3f58d2b64079a1c8e3d5f2a41";
export const BRAM = "c1e84a27f03d95b6e2a8710c4d9f3b5a6e07d12c8f4a93b5e60d1a7c2f84e9b3";
export const CLEO = "5d92b3e8071fa46c9e0d83b2a75c14f6e9087b3d2a51c4f80e96d7a3b1c52e08";
export const DANA = "e07b15c94d2a83f6b1096ce4a5738d2f0c4b91e6a37d5f82c1049be6d3a7f510";
export const HUB = "8b46d2f1a09e537c4b81e60d9a3f75c2e1084d6b93a7f50c2e8d14b6a09f3c75";
export const DOV = "f2084ac6e91d37b50c84a1d7e3f60b92a5c18d4e7f03b629d8a51c7e4f0b3a96";
export const ELI = "194ce7a02b85d3f6914a0e7c5b82d1f36a9e40c7b58d2f13a6e90c4b7d5812f0";
export const FAY = "a6d30f58c1e2749b0583d6e1fa9c47b2305e81d9a4c7f620b3e58d1a9c06f374";

/** The hub's one-time code and address, as `POST /fabric/hub/enroll` accepts them in the mock (case, dashes and spaces ignored). */
export const MOCK_HUB_CODE = "K7M29X4P";
export const MOCK_HUB_ADDRESS = "192.168.1.20:11183";

/**
 * A hub's enrollment text as the daemon builds it (`mesh111::contacts::HubBundle::text`, spec 7.3):
 * `111HUB:<card>:<code>:<address>`, the address last because it may itself contain colons.
 */
export const hubBundleText = (cardText: string, code: string, address: string): string => `111HUB:${cardText}:${code}:${address}`;

const LABEL_ALPHABET = "0123456789abcdefghjkmnpqrstvwxyz";

/** `xxxx-xxxx-xx`: ten characters derived from the id (display only, like the daemon's operator label). */
export function labelOf(hex: string): string {
  let out = "";
  for (let i = 0; i < 10; i++) out += LABEL_ALPHABET[parseInt(hex.slice(i * 2, i * 2 + 2), 16) % 32];
  return `${out.slice(0, 4)}-${out.slice(4, 8)}-${out.slice(8)}`;
}

export function makeContact(node_id: NodeId, over: Partial<Contact> = {}): Contact {
  return {
    node_id,
    bech32: encodeNodeBech32(node_id),
    label: labelOf(node_id),
    alias: null,
    trust: "oob",
    request: false,
    pinned: false,
    has_key: true,
    roles: [],
    home_hub: null,
    ...over,
  };
}

/** How the mock gets a message to a contact. Not part of the contract: it decides which states a send passes through. */
export type MockRoute = "direct" | "mailbox" | "hub_not_yours" | "mailbox_full";

export interface MockOffer {
  code: string;
}

export interface MessagingData {
  contacts: Contact[];
  routes: Record<NodeId, MockRoute>;
  /** Every message of every peer, ascending by `local_seq`. */
  messages: Message[];
  /** `ContactPatch.read_seq` per peer. */
  readMarks: Record<NodeId, number>;
  nextSeq: number;
  nextRecv: number;
  nextMsg: number;
  ownNodeId: NodeId;
  fabric: FabricStatus;
  nearby: NearbySource[];
  panels: HostedPanel[];
  card: Card;
  /** The code the mock hub accepts for `enrollHub`. */
  hubCode: string;
  /** The hub-side offer that is outstanding, if any. */
  offer: MockOffer | null;
  /** Failed enroll attempts; the fifth locks out (429). */
  enrollFailures: number;
  /** The identity holder reports a restore floor: nothing is sealed, outbox items stay `queued` (§4). */
  restorePending: boolean;
  /** Counter for ticket URLs. */
  tickets: number;
}

/** The sentences the mock daemon sends with each `reason_code` (the UI keeps its own copy of the known ones). */
export const MOCK_REASONS: Record<string, string> = {
  waiting_for_key: "Waiting for their key. The message goes out once their card arrives.",
  hub_not_yours: "Their hub is not yours; it is delivered when you meet directly.",
  restore_pending: "Waiting for the identity restore to finish; nothing is sent until it does.",
  mailbox_full: "Your hub's mailbox is full; your copy is kept.",
  store_full: "This node's store is full; new messages wait until space is freed.",
  cancelled: "Cancelled before it left; an empty placeholder goes instead.",
};

/**
 * `POST /messages/{id}/cancel` on the mock's data (spec 4): an own message that is still `queued` and not already cancelled
 * stays in its thread, `queued`, with `reason_code: cancelled`. Returns what happened so the adapter can answer 202 or 409.
 */
export function cancelQueued(m: Message): "cancelled" | "not_own" | "not_queued" {
  if (m.dir !== "own") return "not_own";
  if (m.state !== "queued" || m.reason_code === "cancelled") return "not_queued";
  m.reason_code = "cancelled";
  m.reason = MOCK_REASONS.cancelled ?? null;
  return "cancelled";
}

const CARD_ALPHABET = "abcdefghijklmnopqrstuvwxyz234567";

/** A deterministic 224-character base32-looking card text for a node (the real one is the node's announce, base32). */
export function mockCardText(seedHex: string): string {
  let x = parseInt(seedHex.slice(0, 8), 16) >>> 0 || 1;
  let out = "";
  for (let i = 0; i < 224; i++) {
    x ^= x << 13;
    x >>>= 0;
    x ^= x >>> 17;
    x ^= x << 5;
    x >>>= 0;
    out += CARD_ALPHABET[x % 32];
  }
  return out;
}

const MIN = 60_000;
const HOUR = 60 * MIN;
const iso = (ms: number): string => new Date(ms).toISOString();

/** A deterministic public-key-looking blob of `n` characters, for `NearbyNode.pubkey`. */
const keyOf = (seed: number): string => Array.from({ length: 44 }, (_, i) => CARD_ALPHABET[(seed * 31 + i * 7) % 32]).join("");

export function buildNearby(now: number): NearbySource[] {
  return [
    {
      id: "meshtastic-usb-7c1e0b94a2d3",
      kind: "meshtastic",
      label: "heard via Meshtastic on Bench radio",
      device_id: "usb-7c1e0b94a2d3",
      nodes: [
        {
          id: "!a1b2c3d4",
          num: 2712847316,
          long_name: "Ridge repeater",
          short_name: "RDG1",
          pubkey: keyOf(3),
          snr: 6.5,
          rssi: -92,
          hops_away: 1,
          last_heard_hint: iso(now - 4 * MIN),
          position: { lat: 52.3731, lon: 4.8922, alt_m: 41 },
          channel_utilization: 7.4,
        },
        {
          id: "!0f3e9a12",
          num: 255957522,
          long_name: "Mara's handheld",
          short_name: "MRA",
          pubkey: null,
          snr: -4,
          rssi: -118,
          hops_away: 3,
          last_heard_hint: iso(now - 52 * MIN),
          position: null,
          channel_utilization: null,
        },
        {
          // Hostile-looking text on purpose: names are untrusted RF-origin strings and must render as text only.
          id: "!5be00417",
          num: 1541145623,
          long_name: "<img src=x onerror=alert(1)> Free WiFi",
          short_name: "<b>",
          snr: 0.5,
          rssi: -101,
          hops_away: 0,
          last_heard_hint: null,
          channel_utilization: 2.1,
        },
      ],
    },
  ];
}

/** The declared Meshtastic panel (§13). Enabled and listening only once the service is live. */
export function buildPanels(live: boolean): HostedPanel[] {
  return [
    {
      id: "meshtastic",
      port: 11190,
      entry: "/",
      needs_secure_context: true,
      allow: ["serial", "bluetooth"],
      optional_hosts: ["tile.openstreetmap.org"],
      state: live ? { enabled: true, listening: true, health: "up", dropped: 0 } : { enabled: false, listening: false, health: "unknown" },
      title: "Meshtastic",
      secure_note: "Web Serial and Bluetooth need a secure page: open the dashboard on the node itself, or over HTTPS.",
    },
  ];
}

/** The refusable store classes the daemon reports a quota for (`null` when unset, spec 10.4): never `msg` or `agent_op`. */
export const MOCK_QUOTA_CLASSES = ["kb", "update", "patch", "sync", "object", "custody"] as const;

export function buildFabric(): FabricStatus {
  // `restore_pending` is the identity holder's report (spec 15 as-built); the mock reports "no restore floor" like a healthy node.
  const store: FabricStore = { open: true, used: 3_412_000, watermark: 256_000_000, store_full: false, restore_pending: false };
  return {
    schema: 1,
    links: [
      {
        id: "tcp-lan",
        kind: "tcp-lan",
        state: "up",
        peers: 1,
        detail: null,
        profile: { mtu_bytes: 16384, framing: "stream", reliable: true, broadcast: false, cost_class: "bulk", airtime: null },
      },
    ],
    hub: { enrolled: true, reachable: true, cursor: 41, node_id: HUB, address: MOCK_HUB_ADDRESS, members: 0, offer_active: false },
    custody: { count: 0, bytes: 0 },
    store,
    settings: { hub_role: false, listen_lan: true, custody_opt_in: false, quotas: Object.fromEntries(MOCK_QUOTA_CLASSES.map((c) => [c, null])) },
  };
}

interface Seed {
  peer: NodeId;
  dir: "own" | "in";
  text: string;
  ago: number;
  state?: Message["state"];
  reason?: string | null;
  claimOffset?: number | null;
}

/** Messages in order. Own messages carry their delivery state; incoming ones are always `delivered`. */
function seedMessages(now: number, ownEpoch: number): { messages: Message[]; nextSeq: number; nextRecv: number; nextMsg: number } {
  const seeds: Seed[] = [
    { peer: ADA, dir: "in", text: "Morning! Are you at the workshop today?", ago: 3 * HOUR, claimOffset: -55 * MIN },
    { peer: ADA, dir: "own", text: "Yes, back at 3. Need anything from the shop?", ago: 3 * HOUR - 8 * MIN, state: "delivered" },
    { peer: ADA, dir: "in", text: "Bring the Heltec board and a USB-C cable.\nThe long one, please.", ago: 2 * HOUR + 30 * MIN, claimOffset: 2 * MIN },
    { peer: ADA, dir: "own", text: "Will do.", ago: 2 * HOUR + 20 * MIN, state: "delivered" },
    { peer: ADA, dir: "own", text: "Charging the battery now, 40% so far.", ago: 40 * MIN, state: "relayed" },
    { peer: DANA, dir: "own", text: "Do you still have the photos from the site visit?", ago: 35 * MIN, state: "sent", reason: "mailbox_full" },
    { peer: BRAM, dir: "own", text: "Hi Bram, this is Sam from the workshop.", ago: 30 * MIN, state: "queued", reason: "hub_not_yours" },
    { peer: CLEO, dir: "own", text: "Hello Cleo, I added you from the card.", ago: 20 * MIN, state: "queued", reason: "waiting_for_key" },
    { peer: ADA, dir: "own", text: "On my way back now.", ago: 6 * MIN, state: "sent" },
  ];
  const perPeer: Record<string, number> = {};
  let recv = 0;
  const messages = seeds.map((s, i): Message => {
    const n = (perPeer[s.peer] = (perPeer[s.peer] ?? 0) + 1);
    const at = now - s.ago;
    const reason = s.reason ?? null;
    return {
      id: `msg-01j9zb${(i + 1).toString().padStart(2, "0")}k4q8`,
      peer: s.peer,
      dir: s.dir,
      text: s.text,
      state: s.dir === "in" ? "delivered" : (s.state ?? "queued"),
      reason_code: reason,
      reason: reason ? (MOCK_REASONS[reason] ?? null) : null,
      local_seq: i + 1,
      received_seq: s.dir === "in" ? ++recv : null,
      sent_at_claim: s.dir === "in" && s.claimOffset !== undefined && s.claimOffset !== null ? iso(at + s.claimOffset) : null,
      at: iso(at),
      msg_id: { stream: 0, epoch: ownEpoch, n },
    };
  });
  return { messages, nextSeq: seeds.length + 1, nextRecv: recv + 1, nextMsg: seeds.length + 1 };
}

export function buildMessaging(now: number, ownNodeId: NodeId = "9f2c41d7a0b35e68c1d4f7092ab36e5d8c70f1e24b9a6d3057c8e1f2a4b6d9c3"): MessagingData {
  const contacts: Contact[] = [
    makeContact(ADA, { alias: "Ada (workshop)", trust: "enrolled", pinned: true, home_hub: HUB.slice(0, 16) }),
    makeContact(BRAM, { alias: "Bram", trust: "oob", home_hub: "7c41d2e9aa03b5f1" }),
    makeContact(CLEO, { trust: "oob", has_key: false }),
    makeContact(DANA, { alias: "Dana", trust: "enrolled", home_hub: HUB.slice(0, 16) }),
    makeContact(HUB, { alias: "Workshop hub", trust: "enrolled", roles: ["mailbox", "always_on_hub"], pinned: true }),
  ];
  const seeded = seedMessages(now, 7340);
  return {
    contacts,
    routes: { [ADA]: "mailbox", [BRAM]: "hub_not_yours", [CLEO]: "direct", [DANA]: "mailbox_full", [HUB]: "direct" },
    messages: seeded.messages,
    // Ada's two incoming messages are read; nothing is waiting.
    readMarks: { [ADA]: 3 },
    nextSeq: seeded.nextSeq,
    nextRecv: seeded.nextRecv,
    nextMsg: seeded.nextMsg,
    ownNodeId,
    fabric: buildFabric(),
    nearby: [],
    panels: buildPanels(false),
    card: cardFor(mockCardText(ownNodeId)),
    hubCode: MOCK_HUB_CODE,
    offer: null,
    enrollFailures: 0,
    restorePending: false,
    tickets: 0,
  };
}

// --- Views over the data (the adapter and the tests share these) ---------------------------------------------------

function states(msgs: readonly Message[]): Thread["states"] {
  const out = { queued: 0, sent: 0, relayed: 0, delivered: 0 };
  for (const m of msgs) if (m.dir === "own") out[m.state]++;
  return out;
}

/** Threads, most recent activity first. A peer with messages has a thread even when it is no longer a contact. */
export function threadsOf(d: MessagingData): Thread[] {
  const peers = new Map<NodeId, Message[]>();
  for (const m of d.messages) (peers.get(m.peer) ?? peers.set(m.peer, []).get(m.peer)!).push(m);
  const out: Thread[] = [];
  for (const [peer, msgs] of peers) {
    const contact = d.contacts.find((c) => c.node_id === peer);
    const mark = d.readMarks[peer] ?? 0;
    out.push({
      peer,
      alias: contact?.alias ?? null,
      label: contact?.label ?? labelOf(peer),
      request: contact?.request === true,
      unread: msgs.filter((m) => m.dir === "in" && m.local_seq > mark).length,
      last: msgs[msgs.length - 1] ?? null,
      states: states(msgs),
    });
  }
  return out.sort((a, b) => (b.last?.local_seq ?? 0) - (a.last?.local_seq ?? 0));
}

/** One conversation window as `GET /messages/threads/{peer}` returns it. */
export function pageOf(d: MessagingData, peer: NodeId, sinceSeq: number | undefined, limit: number): { messages: Message[]; more: boolean } {
  const all = d.messages.filter((m) => m.peer === peer);
  if (sinceSeq !== undefined) {
    const newer = all.filter((m) => m.local_seq > sinceSeq);
    return { messages: newer.slice(0, limit), more: newer.length > limit };
  }
  return { messages: all.slice(-limit), more: all.length > limit };
}

// --- Service rows derived from the data (mirrors the registry rules in interface README 7.2 / 111d) ---------------------

export interface ServicePatch {
  state: "live" | "unset" | "error";
  reason_code: string;
  reason: string;
  detail: Record<string, unknown>;
}

const plural = (n: number, one: string, many: string): string => (n === 1 ? `1 ${one}` : `${n} ${many}`);

/** The `fabric` row for a fabric snapshot: LIVE when the store is open and a hub is reachable, this hub has members, or a peer is linked. */
export function fabricRow(f: FabricStatus): ServicePatch {
  const attributed = f.links.reduce((n, l) => n + l.peers, 0);
  const detail = {
    started: true,
    store: { open: f.store.open, full: f.store.store_full },
    hub: { enrolled: f.hub.enrolled, reachable: f.hub.enrolled ? f.hub.reachable : null },
    hub_role: f.settings.hub_role,
    members: f.settings.hub_role ? f.hub.members : null,
    attributed_links: attributed,
  };
  const row = (state: ServicePatch["state"], reason_code: string, reason: string): ServicePatch => ({ state, reason_code, reason, detail });
  if (!f.store.open) return row("error", "store_failed", "The message store could not be opened.");
  if (f.store.store_full) return row("error", "store_full", "The message store is full; new messages cannot be saved until space is freed.");
  if (f.hub.enrolled && f.hub.reachable) return row("live", "hub_reachable", "Your home hub is reachable; messages are delivered or held for you.");
  if (f.hub.members > 0) return row("live", "hub_serving", `This node is a home hub for ${plural(f.hub.members, "member node", "member nodes")}.`);
  if (attributed > 0) return row("live", "peer_linked", `Connected directly to ${plural(attributed, "other 111 node", "other 111 nodes")}.`);
  if (f.settings.hub_role) return row("unset", "hub_waiting", "This node is a home hub, waiting for the first node to enroll.");
  if (f.hub.enrolled) return row("unset", "hub_unreachable", "Your home hub cannot be reached right now; messages wait until it can.");
  return row("unset", "not_enrolled", "No home hub and no other node reachable yet. Set up with the agent to enroll with a hub.");
}

/** The `meshtastic` row once a board is identified and bound and the reader has produced a node DB. */
export function meshtasticLiveRow(boardName: string, nodes: number): ServicePatch & { actions: ("restart" | "log")[] } {
  return {
    state: "live",
    reason_code: "meshtastic_live",
    reason: "meshmud is connected to the board and its list of nodes is in.",
    actions: ["restart", "log"],
    detail: {
      child: "running",
      running: true,
      downlink: { port: 11230, listening: true },
      node_db_present: true,
      board: boardName,
      firmware_version: "2.5.9",
      nodes_heard: nodes,
      channel_utilization: 7.4,
      uplink_lost_sec: null,
      usb_uplink: true,
      port_held_by: null,
      log: "/var/lib/111/log/meshmud.log",
    },
  };
}

/** 64 deterministic hex characters from a string (a stand-in for the NodeId a card's announce carries). */
export function hex64(seed: string): string {
  let out = "";
  for (let k = 0; k < 8; k++) {
    let h = (0x811c9dc5 ^ Math.imul(k + 1, 0x9e3779b1)) >>> 0;
    for (let i = 0; i < seed.length; i++) {
      h ^= seed.charCodeAt(i);
      h = Math.imul(h, 0x01000193) >>> 0;
    }
    h ^= h >>> 15;
    h = Math.imul(h, 0x2c1b3c6d) >>> 0;
    h ^= h >>> 12;
    out += (h >>> 0).toString(16).padStart(8, "0");
  }
  return out;
}
