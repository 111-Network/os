// The `fabric` ("Messaging") service page and the Layer 5 settings page share these words (os/5-messaging/README.md sections
// 8-10, 15, 16): what each link is, what a path can carry, who the home hub is, what the node holds for others, and what the
// store does when it fills. Pure; the views draw it. Everything a peer or a radio could have written goes through
// `cleanPeerText` before it is shown.
import { contactName } from "../../lib/api/contacts";
import { formatBytes, hubSummary } from "../../lib/api/fabric";
import { cleanPeerText } from "../../lib/api/messages";
import { own } from "../../lib/api/own";
import type { Contact, FabricCustody, FabricHub, FabricLink, FabricPathProfile, PairedHub, RelayStatus } from "../../lib/api/types";
import { MAX_NAME_CHARS, plain } from "../nearby/model";
import { gatewayBadge } from "./model";

/** The link kinds of section 8, in words. An unknown kind (a newer daemon) reads "unknown link". */
const LINK_LABEL = {
  "tcp-lan": "Local network (tcp-lan)",
  "kiss-modem": "LoRa modem (kiss-modem)",
  "meshtastic-client": "Meshtastic client",
  "rns-bridge": "Reticulum bridge",
  ble: "Bluetooth (ble)",
} as const;

export const linkLabel = (kind: string): string => (own(LINK_LABEL, kind) ? LINK_LABEL[kind] : "unknown link");

const LINK_STATE = { up: "up", down: "down", degraded: "degraded" } as const;
export const linkStateWord = (state: string): string => (own(LINK_STATE, state) ? LINK_STATE[state] : "unknown");

/** "up to 16 KB per frame, reliable, bulk"; the airtime of a radio path is an advertised physical fact (advanced). */
export function profileSummary(p: FabricPathProfile, advanced = false): string {
  const parts = [`up to ${formatBytes(p.mtu_bytes)} per frame`, p.reliable ? "reliable" : "best effort", p.cost_class === "low_bandwidth" ? "low bandwidth" : "bulk"];
  if (p.broadcast) parts.push("broadcast");
  if (advanced && p.airtime) parts.push(`duty cycle ${(p.airtime.duty_cycle_permille / 10).toFixed(1)}%, ${p.airtime.bits_per_s} bit/s`);
  return parts.join(", ");
}

/** One line for a link: its state, how many peers are attributed on it, and the daemon's one-sentence reason when degraded. */
export function linkLine(l: Pick<FabricLink, "state" | "peers" | "detail">): string {
  const peers = l.state === "up" || l.peers > 0 ? ` · ${l.peers === 1 ? "1 peer" : `${l.peers} peers`}` : "";
  const why = l.detail ? ` · ${cleanPeerText(l.detail).trim()}` : "";
  return `${linkStateWord(l.state)}${peers}${why}`;
}

/**
 * The home hub's name: the contact's alias or label when it is a contact, else its short id. The label is written by the hub
 * (a peer), so it is cleaned, flattened and cut to `MAX_NAME_CHARS`; the view draws it isolated (`PeerText`).
 */
export function hubName(hub: Pick<FabricHub, "node_id">, contacts: readonly Contact[] | null): string | null {
  if (!hub.node_id) return null;
  const c = contacts?.find((x) => x.node_id === hub.node_id);
  return c ? plain(contactName(c), MAX_NAME_CHARS) : `${hub.node_id.slice(0, 8)}…${hub.node_id.slice(-4)}`;
}

/** The home-hub phrase of the page as plain text: the summary, with the hub's name when this node is enrolled. */
export function hubLine(hub: FabricHub, contacts: readonly Contact[] | null): string {
  const base = hubSummary(hub);
  const name = hubName(hub, contacts);
  return hub.enrolled && name ? `${base}: ${name}` : base;
}

/** What this node holds for the nodes that use it as a hub (it cannot read any of it). */
export function custodyText(c: FabricCustody, hubRole: boolean): string {
  if (c.count === 0) return hubRole ? "Holding nothing for anyone right now." : "This node is not acting as a hub, so it holds nothing for anyone.";
  const items = c.count === 1 ? "1 sealed item" : `${c.count} sealed items`;
  return `Holding ${items} (${formatBytes(c.bytes)}) for the nodes that use this one as their hub. They are encrypted for their owners; this node cannot read them.`;
}

// ---------------------------------------------------------------------------------------------- store

export const STORE_FULL_TEXT =
  "The store is full. This node refuses new messages and new mail for others until space is freed. It deletes nothing to make room: undelivered messages, mail it holds for members, pending requests, pinned items and contacts all stay.";
export const STORE_CLOSED_TEXT = "The store is not open, so this node cannot save or send messages. Restarting the node tries to open it again; the agent can look at why it did not open.";
export const STORE_PRUNE_TEXT =
  "When the store passes its high-water mark it frees space in a fixed order: mail it held for others that has already been collected, then old data that newer data replaced. It never deletes a message that has not been delivered, mail it still holds for a member, a pending request, a pinned item or a contact. Your own conversation history is only ever pruned at hard capacity, and only after an export is offered.";

/** The sentence beside the gauge: "3.4 MB of 256 MB". */
export function storeUsageText(used: number, watermark: number): string {
  return `${formatBytes(used)} of ${formatBytes(watermark)} before the store starts freeing space`;
}

// ------------------------------------------------------------------------------------ paired hubs (Layer 7)

/**
 * The largest item one carrier post can pass: ntfy's 4,096 B body holds 3,072 raw bytes, so a put frame is 3,068 B and the item inside it
 * 3,068 - 120 - 11 = 2,937 B (Layer 7 section 6.2). An item over it never goes to a carrier (it can still go over a `tcp` pair).
 */
export const CARRIER_MAX_ITEM_BYTES = 2937;

/** `2937` -> "2,937" (no locale: the digits group the same everywhere). */
export const groupDigits = (n: number): string => String(Math.max(0, Math.floor(Number.isFinite(n) ? n : 0))).replace(/\B(?=(\d{3})+(?!\d))/g, ",");

const PAIR_ROLE = { peer: "peer hub", leaf: "leaf (this hub is its gateway)", gateway: "gateway (it is this hub's upstream)" } as const;
export const pairRoleWord = (role: string): string => (own(PAIR_ROLE, role) ? PAIR_ROLE[role] : "unknown role");

const PAIR_VIA = { tcp: "over the network", carrier: "through the carrier" } as const;
export const pairViaWord = (via: string): string => (own(PAIR_VIA, via) ? PAIR_VIA[via] : "unknown link");

/**
 * How a pair's link is protected (section 12): `tls rpk` (TLS 1.3 raw public keys), `lan` (the owner marked it a LAN or an adopted tunnel,
 * so it runs plain), `plain (warned)` (plain on a non-private address, after the owner accepted the warning). A pair that rides the carrier
 * has no link of its own to protect. An unknown value reads "unknown".
 */
export function pairLinkState(p: Pick<PairedHub, "tls" | "lan" | "via">): string {
  switch (p.tls) {
    case "rpk":
      return "tls rpk";
    case "plain":
      return "plain (warned)";
    case "none":
      return p.via === "carrier" ? "carrier" : p.lan ? "lan" : "unknown";
    default:
      return "unknown";
  }
}

/** The number of items, with its noun: "1 item", "3 items". */
const items = (n: number): string => (n === 1 ? "1 item" : `${groupDigits(n)} items`);

/** What one pair's row says, in order: its role, its link, how it is protected, whether it answers, what waits, and the carrier re-posts. */
export function pairFacts(p: PairedHub): string[] {
  const out = [pairRoleWord(p.role), pairViaWord(p.via), pairLinkState(p), p.reachable ? "reachable" : "not reachable", p.pending === 0 ? "nothing waiting" : `${items(p.pending)} waiting`];
  if (p.via === "carrier" || p.reposts > 0) out.push(p.reposts === 1 ? "re-posted once" : `re-posted ${groupDigits(p.reposts)} times`);
  return out;
}

/** `forward.refused`: this session's count (it restarts at zero with the daemon); the sender keeps its own copy and still reads `relayed` (section 5.6). */
export function refusedText(n: number): string {
  return `${items(n)} refused by every hub they could go to since this node started. The sender still reads relayed and keeps its own copy until it is delivered.`;
}

/** `forward.too_large`: the contract carries a count only, so the copy names the bound, not a per-item size. */
export function tooLargeText(n: number): string {
  return `${items(n)} too large for the carrier since this node started: one carrier post passes an item of up to ${groupDigits(CARRIER_MAX_ITEM_BYTES)} B. The sender keeps its own copy.`;
}

/** The hub's own gateway badge from the relay snapshot (the badge is never a member's view of its hub, section 3.8). */
export const relayBadge = (r: Partial<Pick<RelayStatus, "gateway">>): string => gatewayBadge(r.gateway?.state, r.gateway);
