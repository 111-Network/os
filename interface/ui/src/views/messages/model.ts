// Messages view model (os/messaging/README.md §9, §16): the words and glyphs for delivery evidence, the reason a
// message waits, the "received" time with the sender's claim kept apart, and the one-line summaries of a thread.
// Pure: no React, no clock of its own (every function that needs "now" takes it).
import { parseNodeId } from "../../lib/api/contacts";
import { cleanPeerText } from "../../lib/api/messages";
import { isApiError } from "../../lib/api/problem";
import type { Contact, FabricStatus, Message, MessageState, NodeId, ServiceRecord, Thread } from "../../lib/api/types";

// --- Delivery evidence (§9) -----------------------------------------------------------------------------------------

/** What each state means, in the words of §9. The UI never says more than the evidence supports. */
export const STATE_COPY: Record<MessageState, string> = {
  queued: "In the outbox on this node: there is no path to them yet, their key is missing, or their hub is not yours.",
  sent: "A frame was written to a link. They have not confirmed it yet.",
  relayed: "A mailbox node saved it and will hold it for them.",
  delivered: "Their node acknowledged it. Only that acknowledgement says delivered.",
};

/** Why a message waits or was refused (§9, §15). The API sends the same reasons as `reason_code`; unknown codes fall back to its sentence. */
export const REASON_COPY: Record<string, string> = {
  waiting_for_key: "Waiting for their key. The message goes out once their card arrives.",
  hub_not_yours: "Their hub is not yours; it is delivered when you meet directly.",
  restore_pending: "Waiting for the identity restore to finish. Nothing is sealed or sent until it does.",
  mailbox_full: "Mailbox full: your hub could not take it. Your copy is kept here.",
  store_full: "This node's store is full. New messages wait until space is freed.",
};

export interface StateGlyph {
  /** Evidence steps reached after the outbox: 0 queued, 1 sent, 2 relayed, 3 delivered. */
  filled: 0 | 1 | 2 | 3;
  label: string;
  /** false for a state this UI does not know (a newer daemon): drawn empty and read as "unknown". */
  known: boolean;
}

const GLYPHS: Record<MessageState, StateGlyph> = {
  queued: { filled: 0, label: "queued", known: true },
  sent: { filled: 1, label: "sent", known: true },
  relayed: { filled: 2, label: "relayed", known: true },
  delivered: { filled: 3, label: "delivered", known: true },
};

/** Unknown enum values render as `unknown` (the U1b rule). */
export function glyphFor(state: string): StateGlyph {
  return GLYPHS[state as MessageState] ?? { filled: 0, label: "unknown", known: false };
}

/** The sentence under a state; for an unknown state, that it is unknown. */
export function stateCopy(state: string): string {
  return STATE_COPY[state as MessageState] ?? "This node reports a state this page does not know.";
}

/** Reason text for a message that waits or was refused: this UI's copy for the known codes, the node's own sentence for others, else null. */
export function reasonText(m: Pick<Message, "reason_code" | "reason">): string | null {
  if (m.reason_code && m.reason_code in REASON_COPY) return REASON_COPY[m.reason_code]!;
  const given = m.reason ? cleanPeerText(m.reason).trim() : "";
  return given !== "" ? given : null;
}

/** One status line for the thread view: "relayed: A mailbox node saved it…" plus the reason when there is one. */
export function statusLine(m: Pick<Message, "state" | "reason_code" | "reason">): { label: string; detail: string; reason: string | null } {
  return { label: glyphFor(m.state).label, detail: stateCopy(m.state), reason: reasonText(m) };
}

/** Advanced: the evidence a state rests on, named by the protocol (§6, §9). Derived from the state and reason; the API carries no raw receipts. */
export function evidenceName(m: Pick<Message, "dir" | "state" | "reason_code">): string {
  if (m.dir === "in") return "received here (not acknowledged to the sender until the contact is accepted, if this was a request)";
  switch (m.state) {
    case "queued":
      return "none: still in the outbox";
    case "sent":
      return m.reason_code === "mailbox_full" ? "CUSTODY_ACK REJECT_FULL from the hub" : "frame written to a link";
    case "relayed":
      return "CUSTODY_ACK ACCEPTED from a mailbox-role node for a known member";
    case "delivered":
      return "ACK from the recipient";
    default:
      return "unknown";
  }
}

// --- Names, last line, summaries --------------------------------------------------------------------------------------

export const MAX_LAST_LINE = 80;

/** The alias, else the operator label. The label is display-only, never an authenticator. */
export const threadName = (t: Pick<Thread, "alias" | "label">): string => (t.alias && t.alias.trim() !== "" ? cleanPeerText(t.alias) : t.label);

export const contactTitle = (c: Pick<Contact, "alias" | "label">): string => (c.alias && c.alias.trim() !== "" ? cleanPeerText(c.alias) : c.label);

/** One line of a message for a list: newlines become spaces, long text is cut with an ellipsis. */
export function oneLine(text: string, max = MAX_LAST_LINE): string {
  const flat = cleanPeerText(text).replace(/\s+/g, " ").trim();
  return flat.length > max ? `${flat.slice(0, max - 1).trimEnd()}…` : flat;
}

/** The last line of a thread: "you: …" for an own message, the text for an incoming one, a quiet word when empty. */
export function lastLine(t: Pick<Thread, "last">): string {
  if (!t.last) return "no messages yet";
  const line = oneLine(t.last.text);
  return t.last.dir === "own" ? `you: ${line}` : line;
}

/** "2 queued · 1 sent" for what is still unresolved in a thread; empty when everything is delivered or there is nothing own. */
export function unresolved(states: Thread["states"]): string {
  const parts: string[] = [];
  if (states.queued > 0) parts.push(`${states.queued} queued`);
  if (states.sent > 0) parts.push(`${states.sent} sent`);
  if (states.relayed > 0) parts.push(`${states.relayed} relayed`);
  return parts.join(" · ");
}

/** The state to draw beside a thread: its last message's, if that is an own one; otherwise none. */
export const threadGlyphState = (t: Pick<Thread, "last">): MessageState | null => (t.last && t.last.dir === "own" ? t.last.state : null);

// --- Time (display only; nothing orders by it, §4) ---------------------------------------------------------------------

const DAY = 24 * 60 * 60 * 1000;

function parts(ms: number, timeZone: string | undefined, opts: Intl.DateTimeFormatOptions): string {
  return new Intl.DateTimeFormat("en-GB", { ...opts, ...(timeZone ? { timeZone } : {}) }).format(ms);
}

/**
 * "14:32" today, "Tue 14:32" within the week, "12 Oct 14:32" otherwise, in the reader's local time (or `timeZone`).
 * "unknown" for a missing or broken timestamp.
 */
export function whenText(iso: string | null | undefined, nowMs: number, timeZone?: string): string {
  const t = iso ? Date.parse(iso) : NaN;
  if (!Number.isFinite(t)) return "unknown";
  const hm = parts(t, timeZone, { hour: "2-digit", minute: "2-digit", hourCycle: "h23" });
  const day = (ms: number) => parts(ms, timeZone, { year: "numeric", month: "2-digit", day: "2-digit" });
  if (day(t) === day(nowMs)) return hm;
  if (nowMs - t < 6 * DAY && t < nowMs) return `${parts(t, timeZone, { weekday: "short" })} ${hm}`;
  return `${parts(t, timeZone, { day: "numeric", month: "short" })} ${hm}`;
}

/** The label for a message's own time: incoming were received here; own ones were written here (queued). */
export const timeLabel = (m: Pick<Message, "dir">): string => (m.dir === "in" ? "received" : "written");

/** The sender's claim, apart from the receipt time and marked unverified (§3.3, §4). null when there is none. */
export function claimText(m: Pick<Message, "dir" | "sent_at_claim">, nowMs: number, timeZone?: string): string | null {
  if (m.dir !== "in" || !m.sent_at_claim) return null;
  return `The sender says they wrote this at ${whenText(m.sent_at_claim, nowMs, timeZone)}. That is their own clock and cannot be checked, so messages are ordered by when they arrived here.`;
}

/** "3 unread" for a thread; empty at 0. */
export const unreadText = (n: number): string => (n > 0 ? `${n} unread` : "");

// --- Peer route parameter, ids ---------------------------------------------------------------------------------------

/** The NodeId a `/messages/:peer` segment names (64 hex, or `node1…`), or null when it is neither. */
export function peerFromParam(raw: string | undefined): NodeId | null {
  if (!raw) return null;
  const id = parseNodeId(raw);
  return id.ok ? id.hex : null;
}

/** `3a7f09c1…2a41`: enough to tell two ids apart on a small screen (Advanced shows the whole id). */
export const shortId = (hex: string): string => (hex.length > 14 ? `${hex.slice(0, 8)}…${hex.slice(-4)}` : hex);

// --- The composer ------------------------------------------------------------------------------------------------------

/** A hard ceiling on one draft. The node refuses what Tier 0/1 framing cannot carry (§5) with a 400 of its own. */
export const MAX_COMPOSE_CHARS = 4000;

/** Why a draft cannot be sent, or null. */
export function draftProblem(text: string): string | null {
  if (text.trim() === "") return "Write something first.";
  if (text.length > MAX_COMPOSE_CHARS) return `That is longer than ${MAX_COMPOSE_CHARS} characters.`;
  return null;
}

export type ComposeGate = { ok: true; hint: string | null } | { ok: false; why: string };

/** Can this thread take a new message? Only a real contact can; a pending request is read-only until accepted. */
export function composeGate(contact: Pick<Contact, "request" | "has_key"> | null, pageRequest: boolean): ComposeGate {
  if (!contact) return { ok: false, why: "You can write to your contacts. Add this person from Contacts first." };
  if (contact.request || pageRequest) return { ok: false, why: "This is a message request. Accept it to reply; nothing is sent back to them until you do." };
  return { ok: true, hint: contact.has_key ? null : REASON_COPY.waiting_for_key! };
}

// --- The messaging service line --------------------------------------------------------------------------------------

export interface MessagingBanner {
  kind: "unset" | "error" | "starting";
  text: string;
  /** The agent can help: set up a hub, or free space. */
  fixable: boolean;
}

/**
 * The one honest line above the list when the `fabric` service is not LIVE (interface README 7.2). The sentence is the
 * node's own `reason`; a full store is spelled out because it blocks every send, not just one.
 */
export function messagingBanner(service: Pick<ServiceRecord, "state" | "reason" | "reason_code"> | null | undefined, fabric: Pick<FabricStatus, "store"> | null): MessagingBanner | null {
  if (!service || service.state === "live") return fabric?.store.store_full ? { kind: "error", text: "This node's store is full. New messages wait in the outbox until space is freed.", fixable: true } : null;
  const reason = service.reason.trim();
  if (service.state === "starting") return { kind: "starting", text: reason || "Messaging is starting.", fixable: false };
  if (service.state === "error") return { kind: "error", text: reason || "Messaging has a problem.", fixable: true };
  return { kind: "unset", text: reason || "Messaging is not set up yet.", fixable: true };
}

// --- Errors ------------------------------------------------------------------------------------------------------------

/** One sentence for a failed action: the node's own words when it gave them, a plain fallback otherwise. */
export function errorSentence(err: unknown, fallback: string): string {
  if (!isApiError(err)) return fallback;
  if (err.code === "network") return "Could not reach the node. Check that it is running and try again.";
  return err.problem.detail || err.problem.title || fallback;
}

// --- Requests (INTROs, §7.2) -------------------------------------------------------------------------------------------

/** What to say next to a pending request. Nothing is acknowledged to the sender until the user accepts. */
export const REQUEST_NOTE =
  "Someone you have not added wrote to you. Their words are shown as text only and nothing is sent back until you accept. Their name here is only a label made from their id.";

/** The first lines a request holds, for the card: the thread's last line (the request has no alias to show). */
export const requestPreview = (t: Pick<Thread, "last">): string => (t.last ? oneLine(t.last.text, 140) : "no message held");
