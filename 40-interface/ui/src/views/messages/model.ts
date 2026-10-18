// Messages view model (os/50-messaging/README.md §9, §16): the words and glyphs for delivery evidence, the reason a
// message waits, the "received" time with the sender's claim kept apart, and the one-line summaries of a thread.
// Pure: no React, no clock of its own (every function that needs "now" takes it).
import { parseNodeId } from "../../lib/api/contacts";
import { cleanPeerText, conversationThreads, isCancelled, requestThreads } from "../../lib/api/messages";
import { own } from "../../lib/api/own";
import { isApiError } from "../../lib/api/problem";
import { isIgnored, requestSeq, type IgnoredRequests } from "../../lib/api/requests";
import type { Contact, FabricStatus, Message, MessageState, NodeId, ServiceRecord, Thread } from "../../lib/api/types";
import type { PromptGoal } from "../../lib/prompts";

// --- Delivery evidence (§9) -----------------------------------------------------------------------------------------

/** What each state means, in the words of §9. The UI never says more than the evidence supports. */
export const STATE_COPY: Record<MessageState, string> = {
  queued: "In the outbox on this node: there is no path to them yet, their key is missing, or their hub is not yours.",
  sent: "A frame was written to a link. They have not confirmed it yet.",
  relayed: "A mailbox node saved it and will hold it for them.",
  delivered: "Their node acknowledged it. Only that acknowledgement says delivered.",
};

/**
 * Why a message waits or was refused (§9, §15), by `reason_code`: the wording for the state the code is emitted with
 * (queued, except `mailbox_full`). The API sends the same reasons as `reason`; a code or a (state, code) pair this table
 * does not know falls back to the node's own sentence.
 */
export const REASON_COPY: Record<string, string> = {
  waiting_for_key: "Waiting for their key. The message goes out once their card arrives.",
  hub_not_yours: "Their hub is not yours; it is delivered when you meet directly.",
  restore_pending: "Waiting for the identity restore to finish. Nothing is sealed or sent until it does.",
  mailbox_full: "Mailbox full: your hub could not take it. Your copy is kept here.",
  store_full: "This node's store is full. New messages wait until space is freed.",
  cancelled: "You cancelled this before it left. Its words are not sent; an empty placeholder goes in their place so no message number is skipped.",
};

/**
 * The same code can read differently in another state. A hub that answers CUSTODY_PUT with REJECT_UNKNOWN_DEST leaves
 * the message `sent` with `hub_not_yours` (§9: "your hub does not know them"), where a queued message with that code
 * is waiting for a direct meeting. Keyed (state, code); consulted before `REASON_COPY`.
 */
export const REASON_COPY_BY_STATE: Partial<Record<MessageState, Record<string, string>>> = {
  sent: {
    hub_not_yours: "Your hub does not know them, so it cannot hold this for them. Your copy is kept here; it is delivered when you meet directly.",
  },
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
  return own(GLYPHS, state) ? GLYPHS[state] : { filled: 0, label: "unknown", known: false };
}

/** The sentence under a state; for an unknown state, that it is unknown. */
export function stateCopy(state: string): string {
  return own(STATE_COPY, state) ? STATE_COPY[state] : "This node reports a state this page does not know.";
}

/** What an own message that was cancelled before it left says about itself in place of "in the outbox" (§4: an empty tombstone goes). */
export const CANCELLED_STATE_COPY = "Cancelled by you before it left this node. Nothing of it will be sent.";

/** The details sentence of a message: its state's, or, for a cancelled own message, that it is cancelled. */
export const messageStateCopy = (m: Pick<Message, "dir" | "state" | "reason_code">): string => (isCancelled(m) ? CANCELLED_STATE_COPY : stateCopy(m.state));

/** The sentences around the cancel control (§4). */
export const CANCEL_CONFIRM = "Cancel sending this message";
export const CANCELLED_ANNOUNCEMENT = "Cancelled. It will not be sent; an empty placeholder goes in its place.";
export const CANCEL_UNSUPPORTED_TEXT = "This node cannot cancel a message yet.";

/** Reason text for a message that waits or was refused: this UI's copy for a known (state, code), the node's own sentence for others, else null. */
export function reasonText(m: Pick<Message, "state" | "reason_code" | "reason">): string | null {
  const code = m.reason_code;
  if (code) {
    const byState = own(REASON_COPY_BY_STATE, m.state) ? REASON_COPY_BY_STATE[m.state] : undefined;
    if (byState && own(byState, code)) return byState[code]!;
    if (own(REASON_COPY, code)) return REASON_COPY[code]!;
  }
  const given = m.reason ? cleanPeerText(m.reason).trim() : "";
  return given !== "" ? given : null;
}

/** One status line for the thread view: "relayed: A mailbox node saved it…" plus the reason when there is one. */
export function statusLine(m: Pick<Message, "state" | "reason_code" | "reason">): { label: string; detail: string; reason: string | null } {
  return { label: glyphFor(m.state).label, detail: stateCopy(m.state), reason: reasonText(m) };
}

/**
 * Advanced: what a state means, in the protocol's own names (§6, §9). Derived from the state and reason; the API carries
 * no raw receipts, and none is shown or implied. §16 also lists the `path` a message took under "Advanced appends", but
 * the contract's `Message` has no path field, so none is shown (recorded for C1: the node would have to report it).
 */
export function evidenceName(m: Pick<Message, "dir" | "state" | "reason_code">): string {
  if (m.dir === "in") return "received here (not acknowledged to the sender until the contact is accepted, if this was a request)";
  switch (m.state) {
    case "queued":
      return m.reason_code === "cancelled" ? "none: cancelled before it left (an empty tombstone goes instead)" : "none: still in the outbox";
    case "sent":
      if (m.reason_code === "mailbox_full") return "CUSTODY_ACK REJECT_FULL from the hub";
      if (m.reason_code === "hub_not_yours") return "CUSTODY_ACK REJECT_UNKNOWN_DEST from the hub";
      return "frame written to a link";
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

// --- What the agent is asked, by the node's own reason code ----------------------------------------------------------------

/** Enrolling this node with a home hub (a member's set-up, §15). */
export const SETUP_QUESTION = "Messaging on this node is not working yet. Check why, and walk me through setting it up: enrolling with a home hub, or connecting another 111 node.";
/** A problem with messaging that is already set up (§16 `messaging.fix`: an unreachable home hub, a full store). */
export const FIX_QUESTION = "Messaging on this node has a problem. Check what is wrong (an unreachable home hub, or a full message store) and tell me the steps to fix it.";
/** A node that is a home hub with no members yet (§15: it is offered "Let a node use this hub", not asked to enroll). */
export const OFFER_QUESTION = "This node is a home hub and no node has enrolled with it yet. Walk me through letting another node use this hub: the enrollment code and address to give them, and how they enroll.";

export interface BannerAgent {
  label: string;
  goal: PromptGoal;
  question: string;
  /** The set-up and offer paths are the main action of the banner; a fix is the quieter one. */
  primary: boolean;
}

/**
 * What the banner's agent button asks, chosen by the node's `reason_code` until U2's `messaging.*` goals land. The
 * node reports an unreachable home hub and a hub with no members as NOT SET, but neither is a set-up for an enrolling
 * member: an unreachable hub is a fix (§16), and a hub with no members is offered "Let a node use this hub" (§15).
 */
export function bannerAgent(banner: MessagingBanner, reasonCode: string | null | undefined): BannerAgent | null {
  if (!banner.fixable) return null;
  if (banner.kind === "error" || reasonCode === "hub_unreachable") return { label: "Ask the agent to fix it", goal: "fix", question: FIX_QUESTION, primary: false };
  if (reasonCode === "hub_waiting") return { label: "Let a node use this hub, with the agent", goal: "setup", question: OFFER_QUESTION, primary: true };
  return { label: "Set up messaging with the agent", goal: "setup", question: SETUP_QUESTION, primary: true };
}

// --- Errors ------------------------------------------------------------------------------------------------------------

/** One sentence for a failed action: the node's own words when it gave them, a plain fallback otherwise. */
export function errorSentence(err: unknown, fallback: string): string {
  if (!isApiError(err)) return fallback;
  if (err.code === "network") return "Could not reach the node. Check that it is running and try again.";
  return err.problem.detail || err.problem.title || fallback;
}

// --- Requests (INTROs, §7.2) -------------------------------------------------------------------------------------------

/**
 * What to say next to a pending request. Nothing is acknowledged to the sender until the user accepts. "Ignore" keeps the
 * request pending on the node (§7.2: kept, never auto-pruned, no ACK); only "forget request" discards it.
 */
export const REQUEST_NOTE =
  "Someone you have not added wrote to you. Their words are shown as text only and nothing is sent back until you accept. Ignore moves a request out of the way on this device; it stays pending on the node and nothing is sent. Their name here is only a label made from their id.";

/** Above the folded requests: what ignoring did and what brings a request back. */
export const IGNORED_NOTE =
  "You ignored these on this device. They stay pending on the node and nothing was sent to them, and they are not counted in the badge. A new message from one of them brings its request back.";

/** In an open conversation whose request is ignored on this device. */
export const IGNORED_HERE_NOTE = "You ignored this request on this device. It stays pending on the node and nothing was sent to them. A new message from them brings it back.";

/** The two-step confirmation of "forget request": the DELETE is the only thing that discards the held words, and it is not final. */
export const FORGET_REQUEST_CONFIRM = "forget this request (their held words are discarded; if they write again it comes back)";

/** Spoken after an action on a request. */
export const requestAnnouncement = {
  accepted: (label: string): string => `Accepted the request from ${label}.`,
  ignored: (label: string): string => `Ignored the request from ${label}. It stays pending on the node and nothing was sent.`,
  restored: (label: string): string => `Showing the request from ${label} again.`,
  forgotten: (label: string): string => `Forgot the request from ${label}. Their held words are discarded; if they write again it comes back.`,
};

/** The first lines a request holds, for the card: the thread's last line (the request has no alias to show). */
export const requestPreview = (t: Pick<Thread, "last">): string => (t.last ? oneLine(t.last.text, 140) : "no message held");

/** What the Messages list had last time it told a screen reader something. */
export interface SeenThreads {
  unread: Map<string, number>;
  requests: Map<string, { seq: number; folded: boolean }>;
}

/**
 * What to tell a screen reader about a new thread list, if anything: a new request, a request the user had ignored that
 * wrote again (that is what lifts its fold), or new unread messages. Never about what was already there at load, and
 * not about "show again" or an ignore (the same newest receipt). Returns the state to remember.
 */
export function announceIncoming(prev: SeenThreads | null, threads: readonly Thread[], ignored: IgnoredRequests): { say: string | null; seen: SeenThreads } {
  const unread = new Map(threads.filter((t) => !t.request).map((t) => [t.peer, t.unread] as const));
  const requests = new Map(requestThreads(threads).map((t) => [t.peer, { seq: requestSeq(t), folded: isIgnored(ignored, t.peer, requestSeq(t)) }] as const));
  const seen = { unread, requests };
  if (!prev) return { say: null, seen };
  const fresh = [...requests].some(([peer, r]) => {
    const before = prev.requests.get(peer);
    return !r.folded && (before === undefined || (before.folded && r.seq > before.seq));
  });
  if (fresh) return { say: "New message request.", seen };
  const more = conversationThreads(threads).find((t) => t.unread > (prev.unread.get(t.peer) ?? 0));
  return { say: more ? `New message from ${threadName(more)}.` : null, seen };
}

/** One pending request as the lists draw it. `seq` is its newest `received_seq` (null when its thread is not loaded: then it cannot be ignored). */
export interface RequestItem {
  peer: NodeId;
  label: string;
  preview: string | null;
  seq: number | null;
}

/** A request thread as a list item. */
export const requestItemOf = (t: Pick<Thread, "peer" | "label" | "last">): RequestItem => ({ peer: t.peer, label: t.label, preview: requestPreview(t), seq: requestSeq(t) });
