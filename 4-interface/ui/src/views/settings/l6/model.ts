// Settings, Transport (os/6-transport/README.md §2.1, §6, §8, §10, §11, §15): the words and the small pure rules of the page. No React.
import type { NetworkId, RecipientList } from "../../../lib/api/types";

export const ADAPTERS_HELP =
  "Each network is a separate thing you turn on. While one is off nothing of it runs: no process, no listener, no timer. Meshtastic is part of 111 and costs nothing until a board is bound. The others are separate programs: their size and licence are shown before you install anything, and nothing downloads from here.";

export const ORDER_HELP =
  "Your preferred networks, first is most preferred. It sets the order here and in Messages and what the agent suggests first. In this version the node is on every network you turned on at once; falling back through the list comes later.";

export const REGION_HELP = "The radio region is a fact only you know: the rules differ by country. A Meshtastic board transmits nothing until you set it. The agent may suggest a region, never set one.";
export const REGION_CONFIRM_TEXT = (region: string): string => `Set the region to ${region}. Your board may then transmit on that region's frequencies. Doing this is your confirmation that it is legal where you are`;
export const CALLSIGN_HELP = "Optional, for licensed operators. It is only shown in the confirmation text; 111 never transmits it for you.";

export const POSTURE_HELP =
  "How this node's Reticulum (rnsd) behaves. A change restarts it and its helper in order, is checked for 30 seconds, and goes back by itself if it does not work.";
export const SHARE_INSTANCE_HELP = "Let programs on this node, like the LXMF helper, share this node's Reticulum instance. The helper needs it.";
export const TRANSPORT_HELP = "Route traffic for other Reticulum nodes. A phone that joins this hub needs the hub to route.";
export const LAN_HELP = "Listen for other nodes on your network (a Backbone listener on one TCP port). It exists only while this is on.";
export const IFAC_HELP = "Require an IFAC passphrase from nodes that join. It is the limit on who can use this hub.";

export const JOIN_HELP =
  "Join a phone to this hub over your network. The details below are fetched on this request only, on this node or a paired browser, and are not kept on this page. A passphrase, if there is one, is never put into a QR code.";
export const JOIN_QR_NOTE = "A QR code carries only the address, the port and a one-time pair code; the passphrase is fetched after pairing. This node sends the address and port here as text.";

export const LISTS_HELP =
  "A recipient list is a local group of addresses. Writing to it sends one direct message to each member from this node's bridge identity. It can be a target of a relay but never a source, because there is nothing to read from it.";

export const GATES_HELP =
  "Confirmations the node asks you for in its own words. Each one shows its exact text, and confirming records that text's hash. They are read live: if a fact changes (a board becomes licensed, a channel starts to upload) the node asks again.";
export const HAM_HELP = "On an amateur-licensed board, readable texts and relays through it stay off until you confirm. Sealed 111 messages never go this way.";
export const UPLINK_HELP = "A channel whose board uploads to the internet is off for sending by default, one switch per channel. The switch gates every send on it, relayed copies included.";

export const COMPONENTS_HELP =
  "Optional programs by other people. 111 ships none of their bytes: the install step caches them when it is online, and turning one on installs from that cache, offline. You install them; 111 is not their distributor. Removing files deletes the installed tree and the program's own state; the cached download is a separate choice.";

export const ROUTING_NOTE = "Without IFAC, a hub with transport on and a LAN listener routes other people's Reticulum traffic.";

/** Move the item at `index` one place (`-1` up, `+1` down). Returns the same array when it cannot move. */
export function moveItem<T>(list: readonly T[], index: number, dir: -1 | 1): T[] {
  const to = index + dir;
  if (index < 0 || index >= list.length || to < 0 || to >= list.length) return list as T[];
  const next = list.slice();
  const [it] = next.splice(index, 1);
  next.splice(to, 0, it!);
  return next;
}

/** The order the page shows: the saved order, then any enabled network the saved order does not name (a new adapter appears at the end). */
export function orderWithAll(order: readonly NetworkId[], known: readonly NetworkId[]): NetworkId[] {
  const seen = new Set(order);
  return [...order, ...known.filter((k) => !seen.has(k))];
}

/** "111" in an order is the sealed fabric, not a foreign network. */
export const FABRIC_ID = "111";
export const FABRIC_LABEL = "111 (sealed messages)";

/** A recipient list id from a name: letters, digits, underscore, hyphen; unique among `taken`. */
export function listId(name: string, taken: readonly string[]): string {
  const base = name.toLowerCase().replace(/[^a-z0-9_-]+/g, "-").replace(/^-+|-+$/g, "").slice(0, 40) || "list";
  if (!taken.includes(base)) return base;
  for (let i = 2; i < 1000; i++) if (!taken.includes(`${base}-${i}`)) return `${base}-${i}`;
  return `${base}-${taken.length + 1}`;
}

export const sameLists = (a: readonly RecipientList[], b: readonly RecipientList[]): boolean => JSON.stringify(a) === JSON.stringify(b);
export const MAX_LIST_MEMBERS = 256;
export const MAX_LIST_NAME = 64;
