// The `fabric` ("Messaging") service page and the Layer 5 settings page share these words (os/messaging/README.md sections
// 8-10, 15, 16): what each link is, what a path can carry, who the home hub is, what the node holds for others, and what the
// store does when it fills. Pure; the views draw it. Everything a peer or a radio could have written goes through
// `cleanPeerText` before it is shown.
import { contactName } from "../../lib/api/contacts";
import { formatBytes, hubSummary } from "../../lib/api/fabric";
import { cleanPeerText } from "../../lib/api/messages";
import { own } from "../../lib/api/own";
import type { Contact, FabricCustody, FabricHub, FabricLink, FabricPathProfile } from "../../lib/api/types";

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

/** The home hub's name: the contact's alias or label when it is a contact, else its short id. */
export function hubName(hub: Pick<FabricHub, "node_id">, contacts: readonly Contact[] | null): string | null {
  if (!hub.node_id) return null;
  const c = contacts?.find((x) => x.node_id === hub.node_id);
  return c ? cleanPeerText(contactName(c)) : `${hub.node_id.slice(0, 8)}…${hub.node_id.slice(-4)}`;
}

/** The home-hub phrase of the page: the summary, with the hub's name when this node is enrolled. */
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
  "The store is full. This node refuses new messages and new mail for others until space is freed. Nothing that is still waiting to be delivered, held for a member, pending as a request or pinned is deleted to make room.";
export const STORE_CLOSED_TEXT = "The store is not open, so this node cannot save or send messages. Restarting the node tries to open it again; the agent can look at why it did not open.";
export const STORE_PRUNE_TEXT =
  "When the store passes its high-water mark it frees space in a fixed order: mail it held for others that has already been collected, then old data that newer data replaced. It never deletes a message that has not been delivered, mail it still holds for a member, a pending request or a pinned contact. Your own conversation history is only ever pruned at hard capacity, and only after an export is offered.";

/** The sentence beside the gauge: "3.4 MB of 256 MB". */
export function storeUsageText(used: number, watermark: number): string {
  return `${formatBytes(used)} of ${formatBytes(watermark)} before the store starts freeing space`;
}
