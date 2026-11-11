// Nearby (os/5-messaging/README.md section 14, 16): the nodes a Meshtastic board has heard, per source. Pure: the words for the
// empty states, a node's name and figures as text, and the order. Every string a node reports is an untrusted RF-origin value:
// it is cleaned (`cleanPeerText`), cut, and drawn as text only; nothing here ever builds a link or a prompt from it.
import { cleanPeerText } from "../../lib/api/messages";
import { heardAgoNow, nodeEvidence, nodeName } from "../../lib/api/nearby";
import { ApiError } from "../../lib/api/problem";
import type { RelayAdapter } from "../../lib/api/relay"; // type only: the adapter module stays out of this chunk (reached through getRelay)
import type { DeviceRecord, NearbyEvidence, NearbyNode, NearbySource, RegistryRow, ServiceRecord } from "../../lib/api/types";
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

/** A hint further than this ahead of this node's clock is not "just now": the board's clock (or its owner) is wrong or lying. */
export const CLOCK_AHEAD_MS = 60_000;
export const CLOCK_AHEAD_TEXT = "clock ahead of this node";

/**
 * The node's own report of when it was last heard: "12 min ago" by the node's clock, which is a hint and nothing more. A hint
 * more than a minute in this node's future reads "clock ahead of this node" instead of "just now", so a wrong or hostile clock
 * never makes a node look fresh.
 */
export function heardText(n: Pick<NearbyNode, "last_heard_hint">, now: number): string {
  if (!n.last_heard_hint) return "not reported";
  const at = Date.parse(n.last_heard_hint);
  if (Number.isFinite(at) && at - now > CLOCK_AHEAD_MS) return CLOCK_AHEAD_TEXT;
  return agoText(n.last_heard_hint, now);
}

/** "last heard 4 min ago", or "last heard: clock ahead of this node" (a colon, because that is a remark and not a time). */
export const heardLine = (n: Pick<NearbyNode, "last_heard_hint">, now: number): string => {
  const t = heardText(n, now);
  return t === CLOCK_AHEAD_TEXT ? `last heard: ${t}` : `last heard ${t}`;
};

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
export const STARTING_STALE_TEXT = "Last known list. Meshtastic is starting, so the board's newest list of nodes has not arrived yet and nobody here is confirmed to be in range.";

/** A Meshtastic board's source: not the `111` source and not Reticulum (the page's count line and its board states read only these). */
export const isMeshtasticSource = (s: Pick<NearbySource, "kind">): boolean => s.kind !== "111" && s.kind !== "reticulum";

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
  // The service row is the Meshtastic service's: only a Meshtastic source is read against it. A 111 or Reticulum source (even an empty one) never
  // changes which board state is chosen, and a list without a Meshtastic row carries no Meshtastic stale text.
  const meshSources = (sources ?? []).filter(isMeshtasticSource);
  const meshNodes = meshSources.reduce((n, s) => n + s.nodes.length, 0);
  if (nodes > 0) {
    // While the service restarts the daemon still serves the last snapshot (the board's list has not arrived again): last known.
    const starting = meshNodes > 0 && service?.state === "starting";
    const error = meshNodes > 0 && service?.state === "error";
    return {
      kind: "list",
      stale: disconnected || error || starting,
      text: disconnected ? STALE_TEXT : error ? "The Meshtastic service has a problem, so this is the last list it heard." : starting ? STARTING_STALE_TEXT : null,
    };
  }
  if (sources === null) {
    return ready
      ? { kind: "unavailable", text: "This node did not report who it hears. The list appears here as soon as it does." }
      : { kind: "loading", text: "Waiting for the node to report what it hears." };
  }
  if (service?.state === "error") return { kind: "error", text: (service.reason && service.reason.trim()) || "The Meshtastic service has a problem, so nothing can be heard through it right now." };
  if (service?.state === "starting") return { kind: "starting", text: (service.reason && service.reason.trim()) || "Meshtastic is starting. The board's list of nodes has not arrived yet." };
  if (service?.state === "live" || meshSources.length > 0) {
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

// ------------------------------------------------------------------------------------------ the radar (Layer 7 §10)
//
// Pure geometry and ordering for the radar: no DOM, no clock of its own (the page's clock comes in as an argument). Every string a node
// reports is cleaned and cut before it reaches a dot's label; positions are a node's own claim and only ever shade a cell.

/** The legend, verbatim from §10.1 rule 6. It is always shown with the radar; a test pins it. */
export const RADAR_LEGEND =
  "Rings count hops: the inner ring is heard directly. Direction means nothing unless both nodes shared a position; otherwise a node's place on its ring comes from its id. Shaded areas are positions the sender's radio rounded. The outer ring holds nodes whose hop count is unknown.";

/** Hop rings drawn at most (Meshtastic's own limit is 7 hops, so ring 8); a farther node sits on the last ring and its row says the real count. */
export const MAX_HOP_RINGS = 8;
/** Dots drawn at most: the lists below the radar still hold every node. Nearest first, so the cut drops the farthest. */
export const MAX_RADAR_DOTS = 300;
/** §10.1 rule 4: the coarsest position precision, 10 bits, a cell 46.7 km on a side. v1 knows no finer one. */
export const COARSEST_PRECISION_BITS = 10;
export const COARSEST_CELL_KM = 46.7;

/**
 * The side of a position's uncertainty cell in km for a channel's precision bits. Each bit above 10 halves the side from 46.7 km (13 bits
 * 5.8 km, 16 bits 729 m, 19 bits 91 m, 32 bits 1.1 cm). Anything unknown, fractional or coarser than 10 bits is drawn as the coarsest cell:
 * never a finer cell than the node can show (v1 has no precision from the sidecar, so this is the cell for every position).
 */
export function cellKm(bits: number | null | undefined): number {
  if (!finite(bits) || !Number.isInteger(bits) || bits <= COARSEST_PRECISION_BITS || bits > 32) return COARSEST_CELL_KM;
  return COARSEST_CELL_KM / 2 ** (bits - COARSEST_PRECISION_BITS);
}

/** An uncertainty cell as the radar draws it: the precision it comes from and its side in km. */
export interface RadarCell {
  bits: number;
  km: number;
}

/** A position a node reported: finite and in range, else null. Used for the bearing and for the cell. */
export interface Fix {
  lat: number;
  lon: number;
}
export function validFix(p: { lat?: number; lon?: number } | null | undefined): Fix | null {
  if (!p || !finite(p.lat) || !finite(p.lon) || Math.abs(p.lat) > 90 || Math.abs(p.lon) > 180) return null;
  return { lat: p.lat, lon: p.lon };
}

/** The initial great-circle bearing from `from` to `to`, degrees clockwise from north in [0, 360). Null when the two are the same point. */
export function bearingDeg(from: Fix, to: Fix): number | null {
  const rad = Math.PI / 180;
  const p1 = from.lat * rad;
  const p2 = to.lat * rad;
  const dl = (to.lon - from.lon) * rad;
  const y = Math.sin(dl) * Math.cos(p2);
  const x = Math.cos(p1) * Math.sin(p2) - Math.sin(p1) * Math.cos(p2) * Math.cos(dl);
  if (Math.abs(x) < 1e-12 && Math.abs(y) < 1e-12) return null;
  return (((Math.atan2(y, x) * 180) / Math.PI) % 360 + 360) % 360;
}

/** A stable angle in [0, 360) from an id (FNV-1a over its UTF-16 units): the same id is on the same bearing on every page load. */
export function idAngleDeg(id: string): number {
  let h = 0x811c9dc5;
  for (let i = 0; i < id.length; i++) {
    h ^= id.charCodeAt(i);
    h = Math.imul(h, 0x01000193) >>> 0;
  }
  return (h % 3600) / 10;
}

export type RadarGlyph = "triangle" | "circle" | "diamond" | "square";
/** The glyph follows the source kind: 111 triangle, Meshtastic circle, Reticulum diamond, anything else a square. */
export function glyphFor(kind: string): RadarGlyph {
  return kind === "111" ? "triangle" : kind === "meshtastic" ? "circle" : kind === "reticulum" ? "diamond" : "square";
}
export const GLYPH_KEY = "Marks: triangle is a 111 node, circle a Meshtastic node, diamond a Reticulum node.";

/** Where this node is, when it says so. v1 has no position setting and a 111 node never shares one, so the page passes null. */
export type OwnFix = Fix | null;

export interface RadarDot {
  /** `<source id>/<node id>`: unique on the page. */
  key: string;
  kind: string;
  /** The display name, cleaned and cut. */
  name: string;
  /** Ring 1 = heard directly, ring k = k - 1 hops; null = hop count unknown (the outer ring). Capped at `MAX_HOP_RINGS`. */
  ring: number | null;
  /** The real hop figure when one is known (a capped ring keeps it); null otherwise. */
  hops: number | null;
  /** Place across its ring's band, 0 = inner edge .. 1 = outer edge: stronger SNR (else more recent) sits nearer the inside. */
  depth: number;
  /** Degrees clockwise from north. */
  angle: number;
  /** `bearing` only when this node and the peer both reported a position; `id` otherwise. */
  angleFrom: "bearing" | "id";
  /** The uncertainty cell of a Meshtastic position, else null. */
  cell: RadarCell | null;
  glyph: RadarGlyph;
  snr: number | null;
  /** Seconds since heard, now (the page's clock between frames); null when the row has none. */
  ageS: number | null;
}

export interface RadarModel {
  dots: RadarDot[];
  /** Numbered hop rings drawn (at least 2); the outer "unplaced" ring is one more. */
  rings: number;
  /** A node farther than `MAX_HOP_RINGS` sits on the last ring. */
  capped: boolean;
  /** Nodes left off the radar by `MAX_RADAR_DOTS`. */
  hidden: number;
}

/** The ring of a node: hops + 1 (§10.1 rule 1). A 111 node's comes from its evidence: a LAN link is ring 1, only through the hub ring 2, else unknown. */
function ringOf(kind: string, n: NearbyNode): number | null {
  if (kind === "111") {
    const via = new Set(nodeEvidence(n).map((e) => e.transport));
    return via.has("tcp-lan") ? 1 : via.has("hub") ? 2 : null;
  }
  return finite(n.hops_away) && n.hops_away >= 0 ? Math.floor(n.hops_away) + 1 : null;
}

/** Within a ring: a node with an SNR before one without, the stronger first; then the more recent `heard_ago_s`; then the key. */
function innerFirst(a: { snr: number | null; ageS: number | null; key: string }, b: { snr: number | null; ageS: number | null; key: string }): number {
  if (a.snr !== null && b.snr !== null && a.snr !== b.snr) return b.snr - a.snr;
  if ((a.snr === null) !== (b.snr === null)) return a.snr === null ? 1 : -1;
  if (a.ageS !== b.ageS) {
    if (a.ageS === null) return 1;
    if (b.ageS === null) return -1;
    return a.ageS - b.ageS;
  }
  return a.key.localeCompare(b.key);
}

/**
 * The radar's dots (§10.1). `own` is this node's position (null in v1); `receivedAtMs` is when this page took the snapshot and `nowMs` its
 * clock, so a row's age is the node's `heard_ago_s` plus the time this page has held it (no polling). Pure.
 */
export function radarModel(sources: readonly NearbySource[] | null | undefined, opts: { own?: OwnFix; receivedAtMs: number; nowMs: number }): RadarModel {
  type Draft = Omit<RadarDot, "depth">;
  const drafts: Draft[] = [];
  let capped = false;
  for (const s of sources ?? []) {
    for (const n of s.nodes) {
      let ring = ringOf(s.kind, n);
      const hops = ring === null ? null : ring - 1;
      if (ring !== null && ring > MAX_HOP_RINGS) {
        ring = MAX_HOP_RINGS;
        capped = true;
      }
      const fix = validFix(n.position);
      const own = opts.own ? validFix(opts.own) : null;
      const bearing = own && fix ? bearingDeg(own, fix) : null;
      const key = `${s.id}/${n.id}`;
      drafts.push({
        key,
        kind: s.kind,
        name: displayName(n),
        ring,
        hops,
        angle: bearing ?? idAngleDeg(n.id),
        angleFrom: bearing !== null ? "bearing" : "id",
        cell: fix && s.kind === "meshtastic" ? { bits: usableBits(n.position_precision_bits), km: cellKm(n.position_precision_bits) } : null,
        glyph: glyphFor(s.kind),
        snr: finite(n.snr) ? n.snr : null,
        ageS: heardAgoNow(n, opts.receivedAtMs, opts.nowMs),
      });
    }
  }
  // Nearest ring first (unknown last), then inner-first within a ring; the cut drops the farthest.
  const ringKey = (d: Draft): number => d.ring ?? MAX_HOP_RINGS + 1;
  drafts.sort((a, b) => ringKey(a) - ringKey(b) || innerFirst(a, b));
  const kept = drafts.slice(0, MAX_RADAR_DOTS);
  const placed = kept.reduce((m, d) => (d.ring !== null && d.ring > m ? d.ring : m), 0);
  const perRing = new Map<number, number>();
  for (const d of kept) perRing.set(ringKey(d), (perRing.get(ringKey(d)) ?? 0) + 1);
  const seen = new Map<number, number>();
  const dots: RadarDot[] = kept.map((d) => {
    const k = ringKey(d);
    const i = seen.get(k) ?? 0;
    seen.set(k, i + 1);
    const count = perRing.get(k) ?? 1;
    return { ...d, depth: count === 1 ? 0.5 : 0.2 + (0.6 * i) / (count - 1) };
  });
  return { dots, rings: Math.max(2, placed), capped, hidden: drafts.length - kept.length };
}

/** The precision a cell is drawn at: the channel's when it is a whole number above 10, else the coarsest. */
function usableBits(bits: number | null | undefined): number {
  return finite(bits) && Number.isInteger(bits) && bits > COARSEST_PRECISION_BITS && bits <= 32 ? bits : COARSEST_PRECISION_BITS;
}

/** The ring's name: "heard directly", "1 hop", "3 hops", "hops unknown"; the last numbered ring reads "7+ hops" when the radar capped it. */
export function ringName(ring: number | null, capped = false): string {
  if (ring === null) return "hops unknown";
  if (ring <= 1) return "heard directly";
  if (capped && ring >= MAX_HOP_RINGS) return `${MAX_HOP_RINGS - 1}+ hops`;
  return ring === 2 ? "1 hop" : `${ring - 1} hops`;
}

/** Drawing constants of the radar's 320 x 320 box: the node's marker, the inner edge of ring 1 and the outer edge of the unplaced ring. */
export const RADAR_BOX = { size: 320, centre: 160, inner: 10, outer: 150 } as const;

/** The radius of ring boundary `j` (0 = the node's marker, `rings` = the last hop ring, `rings + 1` = the outer unplaced ring). */
export const ringRadius = (j: number, rings: number): number => RADAR_BOX.inner + ((RADAR_BOX.outer - RADAR_BOX.inner) * j) / (rings + 1);

/** A dot's place in the box: its ring's band, across it by `depth`, at its angle (north up, clockwise). */
export function dotPoint(d: Pick<RadarDot, "ring" | "depth" | "angle">, rings: number): { x: number; y: number } {
  const ring = d.ring ?? rings + 1;
  const lo = ringRadius(ring - 1, rings);
  const hi = ringRadius(ring, rings);
  const r = lo + (hi - lo) * d.depth;
  const a = (d.angle * Math.PI) / 180;
  return { x: RADAR_BOX.centre + r * Math.sin(a), y: RADAR_BOX.centre - r * Math.cos(a) };
}

/** A cell's side on the radar in box units. The radar has no distance scale (it counts hops), so this is a log scale for the eye, 8 .. 30. */
export function cellSidePx(km: number): number {
  const t = Math.log1p(Math.max(0, km)) / Math.log1p(COARSEST_CELL_KM);
  return Math.round(8 + 22 * Math.min(1, t));
}

// ------------------------------------------------------------------------------------------ the radar's buttons

/** What a button press leaves on the page: a sentence, and whether it is a problem. `cause` is the error, for the caller's 401 redirect. */
export interface RadarNotice {
  tone: "info" | "problem";
  text: string;
  cause?: unknown;
}

export const LOOK_STARTED = "Looking on this network for up to 5 seconds. Whatever answers is listed below.";
export const FIND_STARTED = "Looking on this network for your hub for up to 5 seconds. If it answers, it is listed below.";
export const HOPS_STARTED = "Reading Reticulum hop counts. They appear on the radar when the read finishes.";

/** The Layer 7 calls the radar's buttons make (a subset of `RelayAdapter`, so a test passes the mock's). */
export type RadarRelay = Pick<RelayAdapter, "browseRegistry" | "nearbyHops">;

/**
 * The radar's three buttons as plain async functions, one call each: "Look for 111 nodes here" and "Find my hub" are the same
 * `POST /registry/browse` (§7.7; the daemon matches the answers against the home hub's `k`), and the hop read is `POST /nearby/hops`, which
 * only a person starts (§10.1 rule 1). Nothing polls; each press is exactly one request. A failure is the node's own sentence, cleaned.
 */
export function createRadarActions(load: () => Promise<RadarRelay>): { look(): Promise<RadarNotice>; find(): Promise<RadarNotice>; hops(): Promise<RadarNotice> } {
  const run = async (call: (r: RadarRelay) => Promise<unknown>, started: string, fallback: string): Promise<RadarNotice> => {
    try {
      await call(await load());
      return { tone: "info", text: started };
    } catch (err) {
      return { tone: "problem", text: (err instanceof ApiError && plain(err.problem.detail || err.problem.title, 200)) || fallback, cause: err };
    }
  };
  return {
    look: () => run((r) => r.browseRegistry(), LOOK_STARTED, "Could not look for 111 nodes here."),
    find: () => run((r) => r.browseRegistry(), FIND_STARTED, "Could not look for your hub."),
    hops: () => run((r) => r.nearbyHops(), HOPS_STARTED, "Could not read the hop counts."),
  };
}

/** "on this network", "through your hub", "through a carrier": where a 111 node was heard, in our own words (the evidence text is the node's). */
export const EVIDENCE_WHERE: Record<string, string> = { "tcp-lan": "on this network", hub: "through your hub", carrier: "through a carrier" };

/** "on this network: a message from them arrived (verified)": one evidence entry as a row shows it. */
export const evidenceLine = (e: NearbyEvidence): string => `${EVIDENCE_WHERE[e.transport] ?? e.transport}: ${plain(e.text, 120)}`;

// ------------------------------------------------------------------------------------------ registry answers (Layer 7 §7.6)

/** Which layer answered, in the contract's words (RegistryRow.answered_by); a value this build does not know is `unknown`. */
export function answeredByText(a: string): string {
  return a === "hint" ? "hint (unverified)" : a === "signed" ? "signed by this contact" : "unknown";
}

/**
 * One registry row as a line of text. Every string is untrusted and cleaned. Only a `signed` row is a contact's own word; a hint (a DNS-SD
 * record anyone on the LAN can publish) and an unknown layer are presented as what the record claims, with the layer named on every row. A hint
 * that names the home hub says so as unverified: only a signed row is marked "your hub". `ageText` is the page's clock (null: not reported).
 */
export function registryLine(r: Pick<RegistryRow, "label" | "roles" | "answered_by" | "node_id">, hubId: string | null, ageText: string | null): string {
  const signed = r.answered_by === "signed";
  const roles = (Array.isArray(r.roles) ? r.roles : []).filter((x) => typeof x === "string").map((x) => plain(x, 16)).filter((x) => x.length > 0);
  const label = plain(typeof r.label === "string" ? r.label : "", 40);
  const parts = [signed ? label : `${label} (its own claim)`];
  if (roles.length > 0) parts.push(signed ? roles.join(", ") : `roles it claims: ${roles.join(", ")}`);
  parts.push(answeredByText(r.answered_by));
  if (ageText !== null) parts.push(ageText);
  if (hubId !== null && r.node_id === hubId) parts.push(signed ? "your hub" : r.answered_by === "hint" ? "its hint names your hub (not verified)" : "names your hub (not verified)");
  return parts.join(" · ");
}
