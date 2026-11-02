// Nearby slice (os/5-messaging/README.md §14, §15): presence per source. Every string a source reports is an untrusted
// RF-origin value (names truncated to 40 bytes and control characters stripped by the daemon): render as text only, and
// hand it to the agent only inside a labelled untrusted block. A node heard here is never a contact.
import { cleanPeerText } from "./messages";
import { EVIDENCE_TRANSPORTS, type NearbyEvidence, type NearbyNode, type NearbySource } from "./types";

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

// --- Layer 7 (os/7-relay/README.md §2, §10): the `111` source and the evidence behind each row ------------------------------

/** The `111` source: the contacts and paired hubs this node has heard this session (never merged into a Meshtastic or Reticulum row). */
export const isOneOneOneSource = (s: Pick<NearbySource, "kind">): boolean => s.kind === "111";

/**
 * A node's evidence, one entry per transport it was heard on. The text is untrusted and cleaned again here; an entry whose transport is
 * not in the closed set (`tcp-lan`, `hub`, `carrier`, §2.3) is dropped rather than drawn with a guessed meaning. A row without the
 * field (Meshtastic, Reticulum, an older node) has none.
 */
export function nodeEvidence(n: Pick<NearbyNode, "evidence">): NearbyEvidence[] {
  const out: NearbyEvidence[] = [];
  for (const e of n.evidence ?? []) {
    if (typeof e !== "object" || e === null || !(EVIDENCE_TRANSPORTS as readonly string[]).includes(e.transport)) continue;
    const text = cleanPeerText(typeof e.text === "string" ? e.text : "").trim();
    if (text !== "") out.push({ transport: e.transport, text });
  }
  return out;
}

/**
 * Seconds since a node was last heard, now: the node's `heard_ago_s` as of the snapshot plus the time this page has held the snapshot.
 * No timer refreshes ages on the node (§2.3); the page adds its own clock between frames. `null` when the row has no age.
 */
export function heardAgoNow(n: Pick<NearbyNode, "heard_ago_s">, receivedAtMs: number, nowMs: number): number | null {
  if (typeof n.heard_ago_s !== "number") return null;
  return n.heard_ago_s + Math.max(0, Math.floor((nowMs - receivedAtMs) / 1000));
}

/** An age in the words the radar and the rows use. Bucketed for display only: there is no "online" state (§2.1 rule 2). */
export function ageLabel(seconds: number): string {
  if (seconds < 10) return "just now";
  if (seconds < 60) return `${seconds} s ago`;
  if (seconds < 3600) return `${Math.floor(seconds / 60)} min ago`;
  if (seconds < 86400) return `${Math.floor(seconds / 3600)} h ago`;
  return `${Math.floor(seconds / 86400)} d ago`;
}
