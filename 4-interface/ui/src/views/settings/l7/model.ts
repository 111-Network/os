// Settings, Relay & presence (os/7-relay/README.md §12): the words and the small pure rules the page uses. Presence and its bans (§2.4,
// §2.6), the carrier note (§9.5 T6), the pairing bundle and the complement rule (§4.6), the role bits and their costs (§7.2, §12), the
// origin order and the newest release seen (§9.3, §9.5 T4), the metered list (§12). Pure; the views draw it. Every string a hub or a
// server supplied (a label, an address, a url) goes through `plain` before it is shown.
import { formatBytes } from "../../../lib/api/fabric";
import { isApiError } from "../../../lib/api/problem";
import type { MeteredLink, PairRole, PairVia, RelaySettingsPatch, RelayStatus, UpdateOrigin } from "../../../lib/api/types";
import { plain } from "../../nearby/model";
import { OUTSTANDING_NOTE } from "../messaging-model";

// ---------------------------------------------------------------------------------------------------------------- presence

/** The per-transport heartbeat is shown and inert in v1 (§2.4): every write is refused, and the page shows the node's own answer. */
export const PRESENCE_HELP =
  "Presence is learned from what happens: a message arrives, a delivery is confirmed, your hub answers. By default no transport sends a beat of its own. This switch is here so you can see it and find out why it cannot be turned on yet: no transport takes a beat in this version.";

/** §2.6, in the spec's numbers: it is airtime, not CPU, that a beat costs. */
export const AIRTIME_TEXT =
  "Why it is off: one 138 B announce every 15 min is 0.025 % of the channel at SF7 and 0.146 % at SF10 for one node. Fifty nodes announcing every 15 min on SF10 would spend 7.3 % of the channel on “I am here”, and 0.61 % at every 3 h. The big networks already announce far more slowly than that.";

/** §2.4 rule 3: what a beat would never do, once a transport takes one. */
export const HEARTBEAT_BANS = [
  "Never on a band with a duty-cycle limit.",
  "Never on a board marked licensed.",
  "On a phone, never more often than every 9 minutes.",
] as const;

export const intervalText = (min: number): string => (!Number.isFinite(min) || min <= 0 ? "events only" : `every ${Math.floor(min)} min`);

export interface HeartbeatRow {
  transport: string;
  /** "events only", "every 60 min". */
  interval: string;
  /** The current value, for the input. */
  minutes: number;
  /** `layer6` (the Reticulum announce interval) is set elsewhere. */
  elsewhere: boolean;
  owner: string;
}

/** The heartbeat rows of the status, cleaned; a malformed row is dropped. */
export function heartbeatRows(status: Pick<RelayStatus, "presence"> | null): HeartbeatRow[] {
  const rows = status?.presence?.heartbeat;
  if (!Array.isArray(rows)) return [];
  const out: HeartbeatRow[] = [];
  for (const r of rows) {
    if (typeof r?.transport !== "string" || typeof r.interval_min !== "number") continue;
    out.push({ transport: plain(r.transport, 40), interval: intervalText(r.interval_min), minutes: Math.max(0, Math.floor(r.interval_min)), elsewhere: r.owner === "layer6", owner: plain(String(r.owner ?? ""), 20) });
  }
  return out;
}

/**
 * The node's own words for a refusal: the problem's `detail`, verbatim (the heartbeat's 409s carry the reason, and for `rns` the route that
 * does change it), else its title. Never a paraphrase: the heartbeat copy is the node's.
 */
export function problemDetail(err: unknown): string {
  if (isApiError(err)) {
    if (err.code === "network") return "The node could not be reached. Check the connection and try again.";
    return err.problem.detail || err.problem.title || "The node refused the request.";
  }
  return err instanceof Error && err.message ? err.message : "Something went wrong.";
}

// ---------------------------------------------------------------------------------------------------------------- gateway

export const CARRIER_KIND_OPTIONS = [
  { value: "none", label: "none" },
  { value: "ntfy", label: "ntfy" },
] as const;

export const MODE_HELP: Record<"5A" | "5B", string> = {
  "5A": "5A, on events: no connection between sends. A send runs when the network changes, when something waits, or when you press Flush now, then it ends. The default.",
  "5B": "5B, held: one connection stays open to the ntfy server. It uses a thread while on. Not available on a phone.",
};

/** §9.5 T6, S12: `http://` is allowed only for a private address, and the token then crosses that network in clear. */
export const TOKEN_CLEAR_NOTE = "This server address starts with http://, so the access token is sent in clear across your local network. Use https:// when the server has it.";
export const isPlainHttp = (server: string): boolean => /^http:\/\//i.test(server.trim());

/** A write-only secret: only whether one is stored is ever shown. */
export const secretState = (s: { set: boolean } | undefined): string => (s?.set === true ? "set" : "not set");

export const SECRET_HELP = "Write-only: what you type is sent once and never shown again. Leave it empty to keep what is stored.";

/** A gateway needs a configured carrier to mean anything (§3.1); without one, turning it on does nothing visible. */
export const GATEWAY_HELP =
  "A gateway is a hub that carries sealed messages to another hub through a carrier you chose, or over a direct internet link. Nothing runs while the carrier is none or the gateway is off.";

export const OFFER_EARLIER_NOTE = "The pairing code you showed earlier is still valid: it stays valid until you dismiss it or show a new one. Changing the role or link does not cancel it.";

/**
 * The line under the pairing buttons about a code that is not on screen but is still the node's: the one the owner showed before changing the
 * role or link, or one minted earlier or elsewhere (the fabric slice's `offer_active`). A code that is on screen needs no line.
 * The caller passes `earlier` only while the node still holds that code (or does not know yet): a code the node no longer holds gets no line.
 */
export const offerNote = (shown: boolean, earlier: boolean, outstanding: boolean): string | null => (shown ? null : earlier ? OFFER_EARLIER_NOTE : outstanding ? OUTSTANDING_NOTE : null);

export const HUB_ONLY_TEXT = "This needs the hub role: turn it on in Settings, Messaging.";

/**
 * The settings writes of the page, as the patches they send (`PATCH /relay/settings`, openapi RelaySettingsPatch). Sections and tests build
 * them here, so what a test sends is what a control sends. The words of the four consent actions are in views/messages/net/consent.tsx.
 */
export const PATCH = {
  heartbeat: (transport: string, intervalMin: number): RelaySettingsPatch => ({ presence: { heartbeat: [{ transport, interval_min: intervalMin }] } }),
  gatewayEnabled: (enabled: boolean): RelaySettingsPatch => ({ gateway: { enabled } }),
  carrier: (kind: "none" | "ntfy", server: string, mode: "5A" | "5B"): RelaySettingsPatch => ({ carrier: kind === "none" ? { kind } : { kind, server: server.trim(), mode } }),
  secret: (field: "token" | "topic_secret", value: string | null): RelaySettingsPatch => ({ carrier: field === "token" ? { token: value } : { topic_secret: value } }),
  metered: (list: MeteredLink[]): RelaySettingsPatch => ({ metered: list }),
  lanRecord: (on: boolean): RelaySettingsPatch => ({ discovery: { lan_record: on } }),
  gatewayBit: (on: boolean): RelaySettingsPatch => ({ discovery: { public_gateway_bit: on } }),
  offloadBit: (on: boolean): RelaySettingsPatch => ({ discovery: { offload_bit: on } }),
  checkInternet: (on: boolean): RelaySettingsPatch => ({ updates: { check_internet: on } }),
  originServe: (on: boolean): RelaySettingsPatch => ({ origin: { serve: on } }),
  targets: (targets: string[]): RelaySettingsPatch => ({ origin: { targets } }),
};

/**
 * A secret field: only whether something was typed is kept (to enable Save). The text itself lives in the uncontrolled input and nowhere
 * else, so no render ever writes it into a `value` attribute; it is read when Save is pressed and the input is emptied after a
 * successful save.
 */
export interface SecretDraft {
  filled: boolean;
}
export const EMPTY_SECRET: SecretDraft = { filled: false };
export const typeSecret = (_prev: SecretDraft, text: string): SecretDraft => ({ filled: text !== "" });

// ---------------------------------------------------------------------------------------------------------------- pairing

export const ROLES: readonly PairRole[] = ["peer", "leaf", "gateway"];
export const VIAS: readonly PairVia[] = ["tcp", "carrier"];

/** §4.6 row 2: peer pairs with peer, leaf with gateway. */
export const complementOf = (role: PairRole): PairRole => (role === "leaf" ? "gateway" : role === "gateway" ? "leaf" : "peer");

/** What a role means from the side of the hub that names it. */
export const ROLE_MEANING: Record<PairRole, string> = {
  peer: "an equal: each hub forwards for the other",
  leaf: "a leaf: this hub is its gateway and passes its messages on",
  gateway: "a gateway: it is this hub's way out and passes this hub's messages on",
};

export const COMPLEMENT_RULE =
  "The two hubs must name each other's roles the same way round: peer with peer, and leaf with gateway. A leaf hub is reached over the network; a pair through the carrier is always peer with peer. If the picks do not match, nothing is recorded.";

export const PAIR_STEPS =
  "Pairing runs once between two hubs on a network they both reach (the same LAN, a tunnel, or one public address): each shows its bundle and the other scans or pastes it. After that a pair through the carrier talks only through the carrier.";

export const LAN_MARK_HELP =
  "Mark this link as a LAN or an adopted tunnel (a VPN you trust). It then runs without the encryption a link over the open internet gets. Leave it off for a hub on the internet.";

/** The role and link that go together: choosing carrier forces peer; choosing leaf or gateway forces the network link. */
export function settlePick(role: PairRole, via: PairVia, changed: "role" | "via"): { role: PairRole; via: PairVia } {
  if (changed === "via" && via === "carrier") return { role: "peer", via };
  if (changed === "role" && role !== "peer") return { role, via: "tcp" };
  return { role, via };
}

export interface PairBundle {
  card: string;
  code: string;
  spki: string;
  /** `host:port`, or empty: the other hub has no address this one can dial. */
  address: string;
  /** The role the other hub gave this hub. */
  role: PairRole;
  via: PairVia;
}

const BASE32 = /^[a-z2-7]+$/i;
export const BUNDLE_MAX_CHARS = 640;

/**
 * Read a `111PAIR:<card>:<code>:<spki>:<address>:<role>:<via>` text (§4.6 row 1): three fields from the left and two from the right, the
 * address is what remains (a bracketed IPv6 literal may hold `:`). Anything else is null. The node judges the bundle; this only reads what
 * the page needs to propose the matching pick.
 */
export function parsePairBundle(text: string): PairBundle | null {
  const t = text.trim();
  if (t.length === 0 || t.length > BUNDLE_MAX_CHARS || !t.startsWith("111PAIR:") || /\s/.test(t)) return null;
  const parts = t.slice("111PAIR:".length).split(":");
  if (parts.length < 6) return null;
  const via = parts[parts.length - 1]!;
  const role = parts[parts.length - 2]!;
  if (!(ROLES as readonly string[]).includes(role) || !(VIAS as readonly string[]).includes(via)) return null;
  const [card, code, spki] = parts as [string, string, string];
  if (!BASE32.test(card) || code === "" || !BASE32.test(spki)) return null;
  return { card, code, spki, address: parts.slice(3, parts.length - 2).join(":"), role: role as PairRole, via: via as PairVia };
}

/** The same bundle, however the text around it was edited (surrounding space). */
const sameBundle = (a: PairBundle, b: PairBundle): boolean => a.card === b.card && a.code === b.code && a.spki === b.spki && a.address === b.address && a.role === b.role && a.via === b.via;

/**
 * The pick a newly read bundle proposes (the complement of what it gave this hub, over its link), or null: no bundle, or the same bundle
 * as before. A pick the owner made after reading a bundle is never put back by a later edit of the text.
 */
export function pickForBundle(prev: PairBundle | null, next: PairBundle | null): { role: PairRole; via: PairVia } | null {
  if (next === null || (prev !== null && sameBundle(prev, next))) return null;
  return { role: complementOf(next.role), via: next.via };
}

/** What the page tells the person it read, before they press Pair. The address is the other hub's own claim, shown as text. */
export function bundleSummary(b: PairBundle): string {
  const where = b.address === "" ? "no address (it will dial you)" : plain(b.address, 60);
  return `Read a pairing bundle: the other hub gave you the role ${b.role} over ${b.via === "tcp" ? "the network" : "the carrier"}, at ${where}. So pick ${complementOf(b.role)} for it.`;
}

/** Why a pick would not match the bundle, or null when it does (the node still decides). */
export function mismatchNote(b: PairBundle, role: PairRole, via: PairVia): string | null {
  if (via !== b.via) return `The other hub chose the link ${b.via}; pick the same.`;
  if (role !== complementOf(b.role)) return `The other hub chose ${b.role} for you, so its role here is ${complementOf(b.role)}: pick that.`;
  return null;
}

// ---------------------------------------------------------------------------------------------------------------- discovery

export interface DiscoveryBit {
  id: "lan_record" | "public_gateway_bit" | "offload_bit" | "origin";
  label: string;
  /** What it costs, in the words of §7.2 and §12. */
  cost: string;
}

export const DISCOVERY_BITS: readonly DiscoveryBit[] = [
  {
    id: "lan_record",
    label: "LAN record",
    cost: "Announces this node on your local network as _111._tcp so others can find it without a code. It is a hint, not a proof: anyone on the network can read it and anyone could forge one. Needs LAN sharing.",
  },
  {
    id: "public_gateway_bit",
    label: "Gateway bit (role bit 3)",
    cost: "Adds “willing to act as a gateway” to this node's signed card. Everyone who receives the card, on any network, learns that this node has internet. Your hub's members learn it without this. Asks you to confirm.",
  },
  {
    id: "offload_bit",
    label: "Offload bit (role bit 4)",
    cost: "Says this node serves model inference to paired LAN members. One announce is sent when it changes.",
  },
  {
    id: "origin",
    label: "Origin bit (role bit 5)",
    cost: "Set by Serve update files below: it says this hub serves the verified update files. It is not a separate switch.",
  },
];

// ---------------------------------------------------------------------------------------------------------------- updates

export const UPDATE_FROM_COMMAND = "sudo -u node111 111 update --from /media/stick";
export const UPDATE_FROM_HELP =
  "No network at all? Put the three update files on a stick (or any folder, or a web address you typed) and run this on the node. Everything is checked against the 111 release key before it is used, and nothing is installed until the usual update step runs.";

export const INTERNET_CHECK_HELP = "Checks the internet origins once an hour. Off: this node asks no internet server; it can still take an update from your hub or from --from.";
export const SERVE_HELP = "Lets this hub hand its verified update files to the other devices on your network. Needs the hub role and LAN sharing.";

const KIND_WORD: Record<UpdateOrigin["kind"], string> = { hub: "your hub", internet: "the internet", directory: "a folder", url: "a web address" };
export const originKindWord = (kind: string): string => (Object.prototype.hasOwnProperty.call(KIND_WORD, kind) ? KIND_WORD[kind as UpdateOrigin["kind"]] : "unknown");

/** An origin's name for a sentence: your hub, or the host of an internet base, cleaned and cut. */
export function originName(o: Pick<UpdateOrigin, "kind" | "url">): string {
  if (o.kind === "hub") return "your hub";
  try {
    return plain(new URL(o.url).host, 60);
  } catch {
    return plain(o.url, 60);
  }
}

/** What an origin last answered: "never asked", "answered 30 min ago", "failed 2 min ago: <detail>". */
export function answerText(a: UpdateOrigin["last_answer"], agoText: (s: number) => string): string {
  if (a === null || typeof a !== "object") return "not asked yet";
  const ago = typeof a.ago_s === "number" && Number.isFinite(a.ago_s) && a.ago_s >= 0 ? ` ${agoText(a.ago_s)}` : "";
  const detail = typeof a.detail === "string" && a.detail !== "" ? `: ${plain(a.detail, 120)}` : "";
  return a.ok ? `answered${ago}` : `failed${ago}${detail}`;
}

const calverParts = (v: string): number[] | null => {
  const parts = v.trim().split(".");
  if (parts.length < 2 || parts.length > 4) return null;
  const nums = parts.map((p) => (/^\d{1,6}$/.test(p) ? Number(p) : NaN));
  return nums.some(Number.isNaN) ? null : nums;
};

/** Negative when `a` is older than `b`; null when either is not a calver. */
export function compareCalver(a: string, b: string): number | null {
  const x = calverParts(a);
  const y = calverParts(b);
  if (!x || !y) return null;
  for (let i = 0; i < Math.max(x.length, y.length); i++) {
    const d = (x[i] ?? 0) - (y[i] ?? 0);
    if (d !== 0) return d;
  }
  return 0;
}

/** T4 (§9.5): "newest release seen: <calver> from <origin>", the highest calver any origin served and this node verified. */
export function newestSeen(origins: readonly UpdateOrigin[]): { calver: string; from: string } | null {
  let best: { calver: string; origin: UpdateOrigin } | null = null;
  for (const o of origins) {
    const v = o.newest_seen;
    if (typeof v !== "string" || calverParts(v) === null) continue;
    if (best === null || (compareCalver(v, best.calver) ?? 0) > 0) best = { calver: v, origin: o };
  }
  return best ? { calver: plain(best.calver, 24), from: originName(best.origin) } : null;
}

export const newestSeenText = (n: ReturnType<typeof newestSeen>): string => (n ? `newest release seen: ${n.calver} from ${n.from}` : "newest release seen: none yet");

/** A rust target triple as typed: letters, digits, `_` and `-`, three or four parts. */
export const TARGET_TRIPLE = /^[a-z0-9_]+(-[a-z0-9_]+){2,3}$/;

// ---------------------------------------------------------------------------------------------------------------- metered

export const METERED_HELP =
  "Mark a connection that costs you money or data. Carrier bytes are counted over a rolling 24 hours; past the cap nothing is sent until the allowance frees up. A Wi-Fi name is a hint you give: a phone cannot read whether Android treats the link as metered.";

export function meteredLabel(m: MeteredLink): string {
  const key = typeof m.match?.iface === "string" ? "network interface" : "Wi-Fi name";
  const name = plain(m.match?.iface ?? m.match?.ssid ?? "", 40);
  return `${key} ${name}, ${formatBytes(m.daily_cap_bytes)} a day`;
}

export const meteredKey = (m: MeteredLink): string => `${m.match?.iface !== undefined ? "iface" : "ssid"}:${m.match?.iface ?? m.match?.ssid ?? ""}`;

export const CAP_MAX_MB = 1_000_000;

/** A whole number of megabytes (1 MB = 1,000,000 bytes), or null. */
export function parseCapMb(text: string): number | null {
  const t = text.trim();
  if (!/^\d{1,7}$/.test(t)) return null;
  const n = Number(t);
  return n >= 1 && n <= CAP_MAX_MB ? n : null;
}

/** The list with one entry added or its cap changed (an entry is one `iface` or `ssid`); a problem text when the input is not usable. */
export function withMetered(list: readonly MeteredLink[], by: "iface" | "ssid", name: string, capMb: string): { ok: true; list: MeteredLink[] } | { ok: false; problem: string } {
  const n = name.trim();
  if (n === "" || Array.from(n).length > 64 || /[\u0000-\u001f]/.test(n)) return { ok: false, problem: by === "iface" ? "Type the interface name, such as wlan0." : "Type the Wi-Fi name." };
  const mb = parseCapMb(capMb);
  if (mb === null) return { ok: false, problem: "Type the daily cap as a whole number of megabytes, at least 1." };
  const next: MeteredLink = { match: by === "iface" ? { iface: n } : { ssid: n }, daily_cap_bytes: mb * 1_000_000 };
  const key = meteredKey(next);
  const out = list.filter((m) => meteredKey(m) !== key).map((m) => structuredClone(m));
  out.push(next);
  return { ok: true, list: out };
}

export const withoutMetered = (list: readonly MeteredLink[], key: string): MeteredLink[] => list.filter((m) => meteredKey(m) !== key).map((m) => structuredClone(m));
