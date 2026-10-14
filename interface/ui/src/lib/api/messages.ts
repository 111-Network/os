// Messages slice (os/messaging/README.md §4, §9, §15, §16): pure helpers over Thread / Message. The store
// keeps the lists, the mock produces them, the Messages view renders them; none of that logic lives here.
//
// Order is `local_seq`, the node's receipt sequence. `at` and `sent_at_claim` are display only and never
// order anything (§4: no timestamp exists in the envelope).
import type { Message, MessageState, NodeId, Thread } from "./types";

/** One loaded conversation (`GET /messages/threads/{peer}`). */
export interface ThreadPage {
  peer: NodeId;
  /** The peer is an unaccepted message request: the messages are held, read-only, and nothing is acknowledged. */
  request: boolean;
  /** Ascending by `local_seq`. */
  messages: Message[];
  /** Messages exist beyond this window (older ones, when loaded without `since_seq`). */
  more: boolean;
}

/** The limit the contract allows per request (`getThread`). */
export const THREAD_PAGE_LIMIT = 100;
export const THREAD_MAX_LIMIT = 500;

/** Delivery evidence, weakest first (§9). */
export const MESSAGE_STATE_ORDER: readonly MessageState[] = ["queued", "sent", "relayed", "delivered"];

/** Rank of a state in the evidence ladder; an unknown value (a newer daemon) ranks below `queued`. */
export function stateRank(state: string): number {
  return MESSAGE_STATE_ORDER.indexOf(state as MessageState);
}

/**
 * `POST /messages/{id}/retry` answers 409 unless an own message is still `queued` or `sent`
 * (retries otherwise happen on link and contact events, never on timers, §9).
 */
export function isRetryable(m: Pick<Message, "dir" | "state">): boolean {
  return m.dir === "own" && (m.state === "queued" || m.state === "sent");
}

const BIDI_AND_CONTROLS = /[\u0000-\u0008\u000B\u000C\u000E-\u001F\u007F-\u009F\u2028\u2029\u202A-\u202E\u2066-\u2069\uFEFF]/g;

/**
 * Peer-authored text for display (§3.3 / §18): rendered as text only, never as markup. This also drops control
 * characters, line and paragraph separators, the byte-order mark and the explicit bidirectional embeddings, overrides and
 * isolates that let a sender reorder what the reader sees. Newlines and tabs stay, and so do the zero-width joiners and the
 * implicit direction marks that real Persian, Arabic and Hebrew text and emoji sequences need.
 */
export function cleanPeerText(text: string): string {
  return text.replace(BIDI_AND_CONTROLS, "");
}

/** Keep a list ordered by `local_seq` after inserting or replacing `message` (matched by `id`). */
export function upsertMessage(messages: readonly Message[], message: Message): Message[] {
  const at = messages.findIndex((m) => m.id === message.id);
  if (at >= 0) {
    if (JSON.stringify(messages[at]) === JSON.stringify(message)) return messages as Message[];
    const next = messages.slice();
    next[at] = message;
    return next;
  }
  const next = messages.slice();
  let i = next.length;
  while (i > 0 && next[i - 1]!.local_seq > message.local_seq) i--;
  next.splice(i, 0, message);
  return next;
}

/**
 * Fold a `message` frame into a loaded conversation. A conversation that was never loaded stays unloaded
 * (the frame is not enough to show a thread). A frame for a different peer changes nothing.
 */
export function applyMessageToPage(page: ThreadPage | undefined, peer: NodeId, message: Message, change: "new" | "state" | "request"): ThreadPage | undefined {
  if (!page || page.peer !== peer) return page;
  const messages = upsertMessage(page.messages, message);
  const request = change === "request" ? true : page.request;
  if (messages === page.messages && request === page.request) return page;
  return { ...page, request, messages };
}

/** Threads that are pending message requests (INTROs, §7.2), most recent first as the daemon sorted them. */
export const requestThreads = (threads: readonly Thread[]): Thread[] => threads.filter((t) => t.request);

/** Conversations proper: everything that is not a pending request. */
export const conversationThreads = (threads: readonly Thread[]): Thread[] => threads.filter((t) => !t.request);

/** Total unread incoming messages across accepted conversations (requests are counted as requests, not as unread). */
export function unreadTotal(threads: readonly Thread[] | null): number {
  return threads ? threads.reduce((n, t) => (t.request ? n : n + t.unread), 0) : 0;
}

/** The highest `local_seq` of an incoming message, for `ContactPatch.read_seq`; null when none arrived. */
export function newestIncomingSeq(messages: readonly Message[]): number | null {
  let top: number | null = null;
  for (const m of messages) if (m.dir === "in" && (top === null || m.local_seq > top)) top = m.local_seq;
  return top;
}

/** How many own messages in a thread still wait on something (queued) or on the recipient (sent, relayed). */
export function pendingOwn(t: Pick<Thread, "states">): number {
  return t.states.queued + t.states.sent + t.states.relayed;
}
