// Nearby (os/messaging/README.md section 14, 16): the nodes a Meshtastic board has heard, per source. Pure: the words for the
// empty states, a node's name and figures as text, and the order. Every string a node reports is an untrusted RF-origin value:
// it is cleaned (`cleanPeerText`), cut, and drawn as text only; nothing here ever builds a link or a prompt from it.
import { cleanPeerText } from "../../lib/api/messages";
import { nodeName } from "../../lib/api/nearby";
import type { DeviceRecord, NearbyNode, NearbySource, ServiceRecord } from "../../lib/api/types";
import { agoText } from "../services/model";

/** A node's name is cut to this many characters on screen (the daemon already cuts to 40 bytes). */
export const MAX_NAME_CHARS = 40;
/** Short names are four characters in Meshtastic; this is a ceiling for a hostile one. */
export const MAX_SHORT_CHARS = 8;
/** A source heading is the board's name; it is cut too. */
export const MAX_HEADING_CHARS = 80;

/** `s` cut to at most `max` characters (code points, so an emoji is never split) with an ellipsis. */
export function clip(s: string, max: number): string {
  const chars = Array.from(s);
  return chars.length <= max ? s : `${chars.slice(0, Math.max(0, max - 1)).join("").trimEnd()}…`;
}

/** Text for display: control and direction-override characters gone, newlines flattened, cut. */
export const plain = (s: string, max: number): string => clip(cleanPeerText(s).replace(/\s+/g, " ").trim(), max);

/** A node's display name: its long name, else the short one, else its id; cleaned and cut. */
export const displayName = (n: Pick<NearbyNode, "long_name" | "short_name" | "id">): string => plain(nodeName(n), MAX_NAME_CHARS);

/** "RDG1": the short name, only when it adds something to the display name. */
export function shortName(n: Pick<NearbyNode, "long_name" | "short_name" | "id">): string | null {
  const short = plain(n.short_name ?? "", MAX_SHORT_CHARS);
  return short !== "" && short !== displayName(n) ? short : null;
}

const finite = (v: unknown): v is number => typeof v === "number" && Number.isFinite(v);

/** "SNR 6.5 dB, RSSI -92 dBm"; null when the board reported neither. */
export function signalText(n: Pick<NearbyNode, "snr" | "rssi">): string | null {
  const parts: string[] = [];
  if (finite(n.snr)) parts.push(`SNR ${Math.round(n.snr * 10) / 10} dB`);
  if (finite(n.rssi)) parts.push(`RSSI ${Math.round(n.rssi)} dBm`);
  return parts.length > 0 ? parts.join(", ") : null;
}

/** "direct", "1 hop away", "3 hops away"; null when unreported. */
export function hopsText(n: Pick<NearbyNode, "hops_away">): string | null {
  if (!finite(n.hops_away) || n.hops_away < 0) return null;
  const h = Math.floor(n.hops_away);
  return h === 0 ? "direct" : h === 1 ? "1 hop away" : `${h} hops away`;
}

/**
 * A position as text ("52.3731, 4.8922, 41 m"), never a link and never a map: a node's coordinates are its own claim. Null
 * when it is missing or out of range.
 */
export function positionText(n: Pick<NearbyNode, "position">): string | null {
  const p = n.position;
  if (!p || !finite(p.lat) || !finite(p.lon) || Math.abs(p.lat) > 90 || Math.abs(p.lon) > 180) return null;
  const alt = finite(p.alt_m) ? `, ${Math.round(p.alt_m)} m` : "";
  return `${p.lat.toFixed(4)}, ${p.lon.toFixed(4)}${alt}`;
}

/** "7.4% of the channel in use"; null when unreported. */
export function utilizationText(n: Pick<NearbyNode, "channel_utilization">): string | null {
  const u = n.channel_utilization;
  return finite(u) && u >= 0 ? `${Math.round(u * 10) / 10}% of the channel in use` : null;
}

/** The node's own report of when it was last heard: "12 min ago" by the node's clock, which is a hint and nothing more. */
export const heardText = (n: Pick<NearbyNode, "last_heard_hint">, now: number): string => (n.last_heard_hint ? agoText(n.last_heard_hint, now) : "not reported");

/** A public key as the board reports it, shortened (Advanced). */
export const keyText = (n: Pick<NearbyNode, "pubkey">): string | null => (n.pubkey ? plain(n.pubkey, 24) : null);

/**
 * Nearest first (hops away; unknown last), then by name, then by id. The node-clock hint orders nothing: it is a display
 * value, not a freshness proof (section 14).
 */
export function sortedNodes(nodes: readonly NearbyNode[]): NearbyNode[] {
  const hops = (n: NearbyNode): number => (finite(n.hops_away) && n.hops_away >= 0 ? n.hops_away : Number.MAX_SAFE_INTEGER);
  return [...nodes].sort((a, b) => hops(a) - hops(b) || displayName(a).localeCompare(displayName(b)) || a.id.localeCompare(b.id));
}

// ------------------------------------------------------------------------------------------ headings

/**
 * "heard via Meshtastic on Bench radio": the daemon's heading when it gave one, else built from the board's name in the device
 * list, else the plain fallback. Cleaned and cut like everything else a board reports.
 */
export function sourceHeading(src: Pick<NearbySource, "label" | "device_id" | "kind">, devices: readonly DeviceRecord[] | null): string {
  const given = plain(src.label ?? "", MAX_HEADING_CHARS);
  if (given !== "") return given;
  const d = src.device_id ? devices?.find((x) => x.id === src.device_id) : undefined;
  const board = d ? plain(d.alias && d.alias.trim() !== "" ? d.alias : d.name, MAX_HEADING_CHARS - 24) : "";
  return board !== "" ? `heard via Meshtastic on ${board}` : "heard via Meshtastic";
}

// ------------------------------------------------------------------------------------- the page state

export type NearbyState =
  /** Nothing has been read from the node yet. */
  | { kind: "loading"; text: string }
  /** The first read finished and the node did not answer for Nearby (a node without Layer 5, or an error). */
  | { kind: "unavailable"; text: string }
  /** No Meshtastic board is set up on this node: there is nothing to listen with. */
  | { kind: "no_board"; text: string }
  /** A board is identified but not set up. */
  | { kind: "unbound"; text: string }
  /** The service is starting: the board's node list has not arrived. */
  | { kind: "starting"; text: string }
  /** The service has a problem; the list (if any) is the last one heard. */
  | { kind: "error"; text: string }
  /** The board is listening and has heard nobody. */
  | { kind: "empty"; text: string }
  /** There are nodes to show. `stale` says the node is not answering, so the list is the last one it sent. */
  | { kind: "list"; stale: boolean; text: string | null };

export const STALE_TEXT = "Last known list. The node is not answering right now, so nobody here is confirmed to be in range.";

/**
 * Which empty state or list the page shows. `service` is the `meshtastic` registry row (null before the registry arrived or on a
 * node without it); `sources` is `GET /nearby` (null before it was read).
 */
export function nearbyState(
  sources: readonly NearbySource[] | null,
  service: Pick<ServiceRecord, "state" | "reason_code" | "reason"> | null | undefined,
  disconnected: boolean,
  ready = true,
): NearbyState {
  const nodes = (sources ?? []).reduce((n, s) => n + s.nodes.length, 0);
  if (nodes > 0) return { kind: "list", stale: disconnected || service?.state === "error", text: disconnected ? STALE_TEXT : service?.state === "error" ? "The Meshtastic service has a problem, so this is the last list it heard." : null };
  if (sources === null) {
    return ready
      ? { kind: "unavailable", text: "This node did not report who it hears. The list appears here as soon as it does." }
      : { kind: "loading", text: "Waiting for the node to report what it hears." };
  }
  if (service?.state === "error") return { kind: "error", text: (service.reason && service.reason.trim()) || "The Meshtastic service has a problem, so nothing can be heard through it right now." };
  if (service?.state === "starting") return { kind: "starting", text: (service.reason && service.reason.trim()) || "Meshtastic is starting. The board's list of nodes has not arrived yet." };
  if (service?.state === "live" || sources.length > 0) {
    return { kind: "empty", text: "The board is listening and has not heard another Meshtastic node yet. Nodes appear here as soon as one is heard." };
  }
  if (service?.reason_code === "meshtastic_unbound") {
    return { kind: "unbound", text: "A Meshtastic board is identified but not set up yet, so nothing is listening. Set it up from its service page." };
  }
  return { kind: "no_board", text: "No Meshtastic board is set up on this node, so there is nothing to listen with. Plug a board in, then set Meshtastic up from its page." };
}

/** "5 Meshtastic nodes heard through 1 board": the count line above the lists. */
export function heardSummary(sources: readonly NearbySource[]): string {
  const nodes = sources.reduce((n, s) => n + s.nodes.length, 0);
  const boards = sources.length;
  const n = nodes === 1 ? "1 Meshtastic node" : `${nodes} Meshtastic nodes`;
  return `${n} heard through ${boards === 1 ? "1 board" : `${boards} boards`}.`;
}
