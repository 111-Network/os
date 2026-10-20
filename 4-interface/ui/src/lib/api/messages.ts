// Messages slice (os/5-messaging/README.md §4, §9, §15, §16): pure helpers over Thread / Message. The store
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

/** The `reason_code` a message carries once the user cancelled it before it left (§4: an empty tombstone goes instead). */
export const CANCELLED = "cancelled";

/** Was this own message cancelled before it left? It stays in the thread as `queued` with this reason, and nothing more happens to it. */
export const isCancelled = (m: Pick<Message, "dir" | "reason_code">): boolean => m.dir === "own" && m.reason_code === CANCELLED;

/**
 * `POST /messages/{id}/retry` answers 409 unless an own message is still `queued` or `sent`
 * (retries otherwise happen on link and contact events, never on timers, §9). A cancelled message is done: nothing is retried.
 */
export function isRetryable(m: Pick<Message, "dir" | "state"> & Partial<Pick<Message, "reason_code">>): boolean {
  return m.dir === "own" && (m.state === "queued" || m.state === "sent") && m.reason_code !== CANCELLED;
}

/**
 * `POST /messages/{id}/cancel` (§4) works on an own message that is still `queued` and has not been cancelled already: once a
 * frame has been written (`sent`) or a hub took it (`relayed`) the words are out and only the recipient's side is left.
 */
export function isCancellable(m: Pick<Message, "dir" | "state"> & Partial<Pick<Message, "reason_code">>): boolean {
  return m.dir === "own" && m.state === "queued" && m.reason_code !== CANCELLED;
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

/**
 * `cleanPeerText` over every string inside a value (objects, arrays, nested). For a block that is printed as JSON:
 * `JSON.stringify` escapes C0 controls but not U+202E or U+2066, so an override in a peer's text would still reorder
 * the Advanced raw view. Returns a copy; numbers, booleans and null pass through.
 */
export function cleanDeep<T>(value: T): T {
  if (typeof value === "string") return cleanPeerText(value) as T;
  if (Array.isArray(value)) return value.map((v) => cleanDeep(v)) as T;
  if (value && typeof value === "object") return Object.fromEntries(Object.entries(value).map(([k, v]) => [cleanPeerText(k), cleanDeep(v)])) as T;
  return value;
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
 * Keep a window at most `limit` messages: the oldest fall off and `more` says so. A page loaded as "the newest N"
 * stays that window while frames append to it, so a long-lived tab on a chatty thread does not grow without bound.
 */
export function trimPage(page: ThreadPage, limit: number): ThreadPage {
  if (!(limit >= 1) || page.messages.length <= limit) return page;
  return { ...page, more: true, messages: page.messages.slice(page.messages.length - limit) };
}

/**
 * Fold a `message` frame into a loaded conversation. A conversation that was never loaded stays unloaded
 * (the frame is not enough to show a thread). A frame for a different peer changes nothing. The result keeps to the
 * window the page was loaded with (`limit`, the newest N).
 */
export function applyMessageToPage(
  page: ThreadPage | undefined,
  peer: NodeId,
  message: Message,
  change: "new" | "state" | "request",
  limit: number = THREAD_MAX_LIMIT,
): ThreadPage | undefined {
  if (!page || page.peer !== peer) return page;
  const messages = upsertMessage(page.messages, message);
  const request = change === "request" ? true : page.request;
  if (messages === page.messages && request === page.request) return page;
  return trimPage({ ...page, request, messages }, limit);
}

/**
 * Lay a fetched page over the one the store already holds, never going back in time. A response can arrive after a
 * `message` frame that is newer than its snapshot, so: messages the response does not know and that are newer than its
 * newest are kept, and a message never steps down the evidence ladder (a `delivered` stays `delivered` when an older
 * snapshot says `sent`). Everything else is the response's: it is the node's own account of the window.
 */
export function mergeThreadPage(current: ThreadPage | null | undefined, fetched: ThreadPage): ThreadPage {
  if (!current || current.peer !== fetched.peer) return fetched;
  const have = new Map(current.messages.map((m) => [m.id, m] as const));
  const got = new Set(fetched.messages.map((m) => m.id));
  let top = -1;
  for (const m of fetched.messages) if (m.local_seq > top) top = m.local_seq;
  let changed = false;
  const messages = fetched.messages.map((m) => {
    const mine = have.get(m.id);
    if (mine && stateRank(mine.state) > stateRank(m.state)) {
      changed = true;
      return mine;
    }
    return m;
  });
  const newer = current.messages.filter((m) => m.local_seq > top && !got.has(m.id));
  if (!changed && newer.length === 0) return fetched;
  return { ...fetched, messages: [...messages, ...newer].sort((a, b) => a.local_seq - b.local_seq) };
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

/** The highest `received_seq` of an incoming message (what an ignored request is keyed on); null when none arrived. */
export function newestReceivedSeq(messages: readonly Message[]): number | null {
  let top: number | null = null;
  for (const m of messages) if (m.dir === "in" && m.received_seq !== null && (top === null || m.received_seq > top)) top = m.received_seq;
  return top;
}

/** How many own messages in a thread still wait on something (queued) or on the recipient (sent, relayed). */
export function pendingOwn(t: Pick<Thread, "states">): number {
  return t.states.queued + t.states.sent + t.states.relayed;
}
