// Nearby slice (os/messaging/README.md §14, §15): presence per source. Every string a source reports is an untrusted
// RF-origin value (names truncated to 40 bytes and control characters stripped by the daemon): render as text only, and
// hand it to the agent only inside a labelled untrusted block. A node heard here is never a contact.
import { cleanPeerText } from "./messages";
import type { NearbyNode, NearbySource } from "./types";

/** Nodes heard across all sources. */
export const nodesHeard = (sources: readonly NearbySource[] | null | undefined): number => (sources ?? []).reduce((n, s) => n + s.nodes.length, 0);

/**
 * A heard node's name for display: the long name, else the short one, else its id. Cleaned again here because the
 * daemon's stripping is a second line of defence, not the only one.
 */
export function nodeName(n: Pick<NearbyNode, "long_name" | "short_name" | "id">): string {
  const long = cleanPeerText(n.long_name).trim();
  if (long !== "") return long;
  const short = cleanPeerText(n.short_name).trim();
  return short !== "" ? short : cleanPeerText(n.id);
}

/** Replace a source list, keeping the previous object when nothing changed. */
export function sameSources(prev: readonly NearbySource[] | null, next: readonly NearbySource[]): NearbySource[] {
  return prev && JSON.stringify(prev) === JSON.stringify(next) ? (prev as NearbySource[]) : [...next];
}
