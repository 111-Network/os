// Settings -> Messaging & data (os/messaging/README.md sections 7.3, 9, 10, 15, 16): what is typed, scanned and shown on the
// Layer 5 page. Pure, so the flows are tested in the node environment. Nothing here sends anything: the page calls the node's
// own routes and shows what the node answers (never an optimistic value).
import { formatPairCode } from "../../app/pair-code";
import { isApiError } from "../../lib/api/problem";
import { cleanPeerText } from "../../lib/api/messages";
import type { FabricStatus, HubEnrollRequest, HubOffer, ServiceRecord, Thread } from "../../lib/api/types";
import { formatBytes } from "../../lib/api/fabric";
import type { PromptGoal } from "../../lib/prompts";

// ---------------------------------------------------------------------------------------- enroll

/** A one-time enrollment code is 8 characters (section 7.3). */
export const CODE_LENGTH = 8;

/** What the code box holds as the person types: uppercase, dashes and spaces dropped, at most 8, shown `xxxx-xxxx`. */
export const formatCode = formatPairCode;

/** The 8 characters the node is sent: dashes, spaces and case are the node's to ignore, but there is no reason to send them. */
export const normalizeCode = (raw: string): string => raw.replace(/[^A-Za-z0-9]/g, "").toUpperCase().slice(0, CODE_LENGTH);

export const codeComplete = (raw: string): boolean => normalizeCode(raw).length === CODE_LENGTH;

const HOST = /^(?:\[[0-9A-Fa-f:.]+\]|[A-Za-z0-9](?:[A-Za-z0-9.-]*[A-Za-z0-9])?)$/;

export type AddressResult = { ok: true; address: string } | { ok: false; text: string };

/** `host:port` of a hub's tcp-lan listener: a name, an IPv4 address or a bracketed IPv6 one, and a port. The node checks it again. */
export function parseAddress(raw: string): AddressResult {
  const t = raw.trim();
  if (t === "") return { ok: false, text: "Type the hub's address, like 192.168.1.20:11183." };
  if (/^[A-Za-z][A-Za-z0-9+.-]*:\/\//.test(t) || t.includes("/")) return { ok: false, text: "Type only the address and port, like 192.168.1.20:11183, without http:// or a path." };
  const cut = t.lastIndexOf(":");
  if (cut <= 0) return { ok: false, text: "The address needs a port, like 192.168.1.20:11183." };
  const host = t.slice(0, cut);
  const port = t.slice(cut + 1);
  if (!HOST.test(host) || host.length > 253) return { ok: false, text: "That is not a host name or IP address." };
  if (!/^\d{1,5}$/.test(port) || Number(port) < 1 || Number(port) > 65535) return { ok: false, text: "The port is a number from 1 to 65535." };
  return { ok: true, address: t };
}

/** What a hub's enrollment text holds: its card, the one-time code, and (when it knows one) its address. */
export interface EnrollBundle {
  /** The hub's card text, which the member needs to seal its request when the hub is not yet a contact. */
  card: string;
  code: string;
  /** `host:port`; empty when the hub did not know its network address yet (the person types it). */
  address: string;
}

/** The prefix of a hub's enrollment text (`mesh111::contacts::BUNDLE_PREFIX`). */
export const BUNDLE_PREFIX = "111HUB:";
/** The daemon's bound on a card plus its framing (`MAX_CARD_TEXT` 512 + 128). */
const BUNDLE_MAX_CHARS = 640;

/**
 * Read a hub's enrollment text, what its QR holds or what is pasted: `111HUB:<card>:<code>:<address>` (section 7.3; the prefix's case
 * does not matter, and the address, which may itself contain colons, comes last and may be empty). The page only checks the shape
 * and that the address could be one; the node decides whether the card and the code are good. Returns null for anything else, and
 * never throws.
 */
export function parseEnrollBundle(text: string): EnrollBundle | null {
  const t = text.trim();
  if (t.length > BUNDLE_MAX_CHARS || t.slice(0, BUNDLE_PREFIX.length).toUpperCase() !== BUNDLE_PREFIX) return null;
  const [card, rawCode, ...rest] = t.slice(BUNDLE_PREFIX.length).split(":");
  if (rest.length === 0 || !card || !/^[A-Za-z0-9-]{16,}$/.test(card) || !rawCode || !/^[A-Za-z0-9-]{8,9}$/.test(rawCode)) return null;
  const code = normalizeCode(rawCode);
  if (code.length !== CODE_LENGTH) return null;
  const raw = rest.join(":");
  if (raw.trim() === "") return { card, code, address: "" };
  const address = parseAddress(raw);
  return address.ok ? { card, code, address: address.address } : null;
}

export type EnrollRequest = { ok: true; body: HubEnrollRequest } | { ok: false; text: string };

/** The body of `POST /fabric/hub/enroll`, or the sentence that says what is missing. The card rides along when it was scanned or pasted. */
export function enrollRequest(address: string, code: string, card: string | null): EnrollRequest {
  const a = parseAddress(address);
  if (!a.ok) return { ok: false, text: a.text };
  if (!codeComplete(code)) return { ok: false, text: "The code is 8 letters and digits, like K7M2-9X4P." };
  const body: HubEnrollRequest = { address: a.address, code: normalizeCode(code) };
  if (card) body.card = card;
  return { ok: true, body };
}

/** One sentence for a failed enroll, by what the node answered (section 7.3, interface contract `enrollHub`). */
export function enrollErrorText(err: unknown): string {
  if (!isApiError(err)) return err instanceof Error && err.message ? err.message : "Something went wrong.";
  if (err.code === "network") return "The node could not be reached. Check the connection and try again.";
  if (err.status === 409) return "This node already has a home hub. A node has one home hub in this version; forget the current one in Contacts first.";
  if (err.code === "rate_limited" || err.status === 429) return "The hub locked this code after five wrong tries. Ask its owner for a new one.";
  if (err.status === 502 || err.code === "upstream_error") {
    return "The hub could not be reached, or it did not accept that code. Check the address and the code, that the hub is on, and that you are on the same network.";
  }
  if (err.status === 400) return err.problem.detail || "The address or the code is not valid. Check both and try again.";
  return err.problem.detail || err.problem.title || "The node refused the request.";
}

export const ENROLLED_TEXT = "Enrolled. This node will hand its messages to the hub when the people it writes to are away, and collect what the hub holds for it.";

// -------------------------------------------------------------------------------------------- offer

/** The code as it is shown (`xxxx-xxxx`). */
export const offerCodeText = (offer: Pick<HubOffer, "code">): string => formatCode(offer.code);

/** The address line under the code. A hub with no LAN address cannot be reached by another node yet. */
export function offerAddressText(offer: Pick<HubOffer, "address">): string {
  return offer.address ? cleanPeerText(offer.address) : "This hub does not know its network address yet. Turn on the LAN listener so another node can reach it.";
}

export const OFFER_NOTE =
  "Give the other node the QR code, or send it the text. The code works once. After five wrong tries the hub locks it. It is shown here only now: dismiss it when you are done, or when it is lost.";

export const OUTSTANDING_NOTE =
  "A code is outstanding, but it was shown once and cannot be shown again. Dismiss it, or show a new one (that cancels this one).";

// ------------------------------------------------------------------------------------------ settings

/** The three switches of `PATCH /fabric/settings`. */
export type SettingKey = "hub_role" | "listen_lan" | "custody_opt_in";

export const HUB_ROLE_HELP =
  "Let other nodes use this one as their home hub: it holds their messages while they are away, sealed so it cannot read them. Only nodes you enroll can use it.";
export const LAN_LISTEN_HELP =
  "Accept connections from other 111 nodes on your local network. Between nodes the link itself is not encrypted in this version: someone on the same network can see which nodes talk to this one and how much, but not what the messages say.";
export const CUSTODY_HELP =
  "Hold mail for the nodes that use this one as a hub even if this is a phone or a laptop that is not always on. A node meant to stay on (a hub-category node) does this by default.";

/** Spoken after a setting changed, from the value the node answered. */
export const settingAnnouncement = (label: string, on: boolean): string => `${label} ${on ? "on" : "off"}.`;

// ------------------------------------------------------------------------------------------- quotas

/** The stored-data classes of section 10.4 that have a quota (capacity shaping; per-sender limits do not exist). */
export const QUOTA_CLASSES: ReadonlyArray<{ id: string; label: string }> = [
  { id: "msg", label: "Messages" },
  { id: "agent_op", label: "Agent operations" },
  { id: "kb", label: "Knowledge base" },
  { id: "update", label: "Updates" },
  { id: "patch", label: "Patches" },
];

const MB = 1_000_000;

export interface QuotaRow {
  id: string;
  label: string;
  /** The quota in megabytes as the box shows it; empty means no quota. */
  mb: string;
}

const mbText = (bytes: number): string => String(Math.round((bytes / MB) * 100) / 100);

/** The known classes first, then any other class the node reports (named as it names it, cleaned). */
export function quotaRows(quotas: Readonly<Record<string, number | null | undefined>>): QuotaRow[] {
  const known = new Set(QUOTA_CLASSES.map((c) => c.id));
  const rows: QuotaRow[] = QUOTA_CLASSES.map((c) => ({ id: c.id, label: c.label, mb: typeof quotas[c.id] === "number" ? mbText(quotas[c.id] as number) : "" }));
  for (const [id, v] of Object.entries(quotas)) {
    if (!known.has(id) && typeof v === "number") rows.push({ id, label: cleanPeerText(id).slice(0, 40), mb: mbText(v) });
  }
  return rows;
}

export type QuotaParse = { ok: true; bytes: number | null } | { ok: false; text: string };

/** A box in megabytes: empty is "no quota" (null), a non-negative number is that many bytes. */
export function parseQuota(raw: string): QuotaParse {
  const t = raw.trim();
  if (t === "") return { ok: true, bytes: null };
  if (!/^\d+(\.\d+)?$/.test(t)) return { ok: false, text: "A quota is a number of megabytes, or empty for none." };
  const bytes = Math.round(Number(t) * MB);
  return Number.isSafeInteger(bytes) ? { ok: true, bytes } : { ok: false, text: "That quota is too large." };
}

/** The `quotas` object of the PATCH: only the classes whose box differs from what the node holds. Null when nothing changed or a box is not a number. */
export function quotaPatch(
  current: Readonly<Record<string, number | null | undefined>>,
  boxes: Readonly<Record<string, string>>,
): { ok: true; quotas: Record<string, number | null> | null } | { ok: false; text: string } {
  const out: Record<string, number | null> = {};
  for (const [id, raw] of Object.entries(boxes)) {
    const parsed = parseQuota(raw);
    if (!parsed.ok) return { ok: false, text: parsed.text };
    const now = typeof current[id] === "number" ? (current[id] as number) : null;
    if (parsed.bytes !== now) out[id] = parsed.bytes;
  }
  return { ok: true, quotas: Object.keys(out).length > 0 ? out : null };
}

/** "No quota" or "about 50 MB". */
export const quotaText = (bytes: number | null | undefined): string => (typeof bytes === "number" ? `about ${formatBytes(bytes)}` : "no quota");

// ------------------------------------------------------------------------------------------ restore

export const RESTORE_HELP =
  "If this node was restored from its 12 words, it waits before it sends anything: it must be sure it never reuses a message number the old device already used. Messages you write meanwhile stay queued, with the reason shown, and go out by themselves when it is done.";
export const RESTORE_PENDING_TEXT = "A restore is finishing on this node. Nothing is sealed or sent until it is done; what you write waits in the outbox.";

/** The node reports a restore floor: the `fabric` row says so, or a message is queued for it (section 4, `restore_pending`). */
export function restorePending(service: Pick<ServiceRecord, "reason_code"> | null | undefined, threads: readonly Thread[] | null): boolean {
  if (service?.reason_code === "restore_pending") return true;
  return (threads ?? []).some((t) => t.last?.dir === "own" && t.last.state === "queued" && t.last.reason_code === "restore_pending");
}

// -------------------------------------------------------------------------------------------- export

export const EXPORT_HELP =
  "An encrypted export of your message history (an age file) is planned; it is not built yet. Until it lands, your history lives in the node's encrypted store. Back up your 12 words (Essentials), and use the command below to see what the store holds.";
export const EXPORT_COMMAND = "111 fabric status";

// -------------------------------------------------------------------------------------- the agent

export interface FabricAgent {
  goal: PromptGoal;
  label: string;
  /** The agent is the main action: messaging does not work yet. */
  primary: boolean;
}

/**
 * What "ask the agent" offers on this page: a fix when messaging has a problem (a full or closed store, an unreachable home
 * hub), a set-up when it is not set up, nothing special when it works (the generic question stays).
 */
export function fabricAgent(service: Pick<ServiceRecord, "state" | "reason_code"> | null | undefined, fabric: Pick<FabricStatus, "store"> | null): FabricAgent | null {
  if (fabric && (fabric.store.store_full || !fabric.store.open)) return { goal: "messaging.fix", label: "Ask the agent to fix it", primary: true };
  if (!service) return null;
  if (service.state === "error" || service.reason_code === "hub_unreachable") return { goal: "messaging.fix", label: "Ask the agent to fix it", primary: true };
  if (service.state === "unset") return { goal: "messaging.setup", label: "Set up messaging with the agent", primary: true };
  return null;
}
