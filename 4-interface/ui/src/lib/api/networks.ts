// Foreign networks slice (os/6-transport/README.md §3, §4, §9, §10): pure helpers over Network / ForeignConversation /
// ForeignMessage. The store keeps the lists, the mock produces them, the net views render them; none of that logic lives here.
//
// Order is `seq`, the node's receipt sequence. `at` and `claim_ts` are display only and order nothing. Every foreign string
// (a title, a sender name, a body, an account) is untrusted: it is drawn as text and never becomes a link or a prompt.
import { own } from "./own";
import type { ForeignConversation, ForeignMessage, ForeignThread, Network, NetworkId } from "./types";

/** The window sizes the contract allows per request (`getForeignConversation`). */
export const FOREIGN_PAGE_LIMIT = 100;
export const FOREIGN_MAX_LIMIT = 500;

/** One loaded conversation: `GET /networks/{net}/conversations/{conv}` without its envelope. */
export type ForeignPage = Pick<ForeignThread, "conversation" | "messages" | "more">;

/**
 * Delivery evidence, weakest first (§4). `failed` ranks below `queued`: the lowest state among the parts of a split send is what
 * the person reads, and a failed part makes the whole send incomplete. A newer daemon's unknown state ranks nowhere (-1).
 */
export const FOREIGN_STATE_ORDER = ["failed", "queued", "sent", "relayed", "delivered"] as const;

export const foreignRank = (state: string): number => (FOREIGN_STATE_ORDER as readonly string[]).indexOf(state);

/** The weakest of these states; `unknown` when one of them is not in the closed enum (the U1b rule) or the list is empty. */
export function lowestState(states: readonly string[]): string {
  let low: string | null = null;
  let lowRank = Infinity;
  for (const s of states) {
    const r = foreignRank(s);
    if (r < 0) return "unknown";
    if (r < lowRank) {
      low = s;
      lowRank = r;
    }
  }
  return low ?? "unknown";
}

/** The `reason_code` a row carries once the person cancelled it before it left. */
export const FOREIGN_CANCELLED = "cancelled";
/**
 * A row whose hand-off to the carrier started and neither an acceptance nor a refusal came back (§4 at most once): after a crash it
 * is never re-sent on its own and reads "may have been sent; resend?". The contract has no field for the store's `meta.handoff`, so
 * the daemon is asked to publish it as this `reason_code`; a daemon that does not simply never shows the question.
 */
export const HANDOFF_PENDING = "handoff_pending";

export const isHandoffPending = (m: Pick<ForeignMessage, "dir" | "reason_code">): boolean => m.dir === "own" && m.reason_code === HANDOFF_PENDING;

/** Cancel drops a row that is still `queued` with nothing sent (`Fabric::cancel`); a cancelled row is done. */
export const isForeignCancellable = (m: Pick<ForeignMessage, "dir" | "state" | "reason_code">): boolean =>
  m.dir === "own" && m.state === "queued" && m.reason_code !== FOREIGN_CANCELLED && m.reason_code !== HANDOFF_PENDING;

/** The person may ask again for a row still waiting, handed over without proof, or refused before air (`Retry::Never` is the user's to retry). */
export const isForeignRetryable = (m: Pick<ForeignMessage, "dir" | "state" | "reason_code">): boolean =>
  m.dir === "own" && (m.state === "queued" || m.state === "sent" || m.state === "failed") && m.reason_code !== FOREIGN_CANCELLED;

/** Keep a list ordered by `seq` after inserting or replacing `m` (matched by `id`); a stale snapshot never steps a row down the ladder. */
export function upsertForeign(messages: readonly ForeignMessage[], m: ForeignMessage): ForeignMessage[] {
  const at = messages.findIndex((x) => x.id === m.id);
  if (at >= 0) {
    const mine = messages[at]!;
    if (foreignRank(mine.state) > foreignRank(m.state) && foreignRank(m.state) >= 0) return messages as ForeignMessage[];
    if (JSON.stringify(mine) === JSON.stringify(m)) return messages as ForeignMessage[];
    const next = messages.slice();
    next[at] = m;
    return next;
  }
  const next = messages.slice();
  let i = next.length;
  while (i > 0 && next[i - 1]!.seq > m.seq) i--;
  next.splice(i, 0, m);
  return next;
}

/** Keep a window at most `limit` rows: the oldest fall off and `more` says so. */
export function trimForeign(page: ForeignPage, limit: number): ForeignPage {
  if (!(limit >= 1) || page.messages.length <= limit) return page;
  return { ...page, more: true, messages: page.messages.slice(page.messages.length - limit) };
}

/** The highest `seq` held in a page (0 when empty). */
export const topSeq = (messages: readonly ForeignMessage[]): number => messages.reduce((n, m) => (m.seq > n ? m.seq : n), 0);

/**
 * Lay a fetched window over the one the store holds, never going back in time: rows the response does not know and that are newer
 * than its newest are kept, and a row never steps down the evidence ladder. Everything else is the response's: it is the node's
 * own account of the window (a cancelled row is gone from it).
 */
export function mergeForeignPage(current: ForeignPage | null | undefined, fetched: ForeignPage): ForeignPage {
  if (!current || current.conversation.id !== fetched.conversation.id) return fetched;
  const have = new Map(current.messages.map((m) => [m.id, m] as const));
  const got = new Set(fetched.messages.map((m) => m.id));
  const top = topSeq(fetched.messages);
  let changed = false;
  const messages = fetched.messages.map((m) => {
    const mine = have.get(m.id);
    if (mine && foreignRank(mine.state) > foreignRank(m.state) && foreignRank(m.state) >= 0) {
      changed = true;
      return mine;
    }
    return m;
  });
  const newer = current.messages.filter((m) => m.seq > top && !got.has(m.id));
  if (!changed && newer.length === 0) return fetched;
  return { ...fetched, messages: [...messages, ...newer].sort((a, b) => a.seq - b.seq) };
}

/** Fold a `since_seq` tail into the held page: new rows are inserted, known rows are replaced (monotone), the conversation record is the newest. */
export function applyForeignTail(current: ForeignPage | null | undefined, tail: ForeignPage): ForeignPage {
  if (!current || current.conversation.id !== tail.conversation.id) return tail;
  let messages = current.messages as ForeignMessage[];
  for (const m of tail.messages) messages = upsertForeign(messages, m);
  return { conversation: tail.conversation, messages, more: current.more };
}

// --- Networks -------------------------------------------------------------------------------------------------------

/** A network is "enabled" for the rail and the fan-out picker unless its optional component is switched off (`disabled`). */
export const networkEnabled = (n: Pick<Network, "state">): boolean => n.state !== "disabled";

export const enabledNetworks = (networks: readonly Network[] | null | undefined): Network[] => (networks ?? []).filter(networkEnabled);

/** The networks in the preferred order (`order` first, in its order; the rest as the node listed them). `111` in the order is the fabric and has no `Network`. */
export function orderedNetworks(networks: readonly Network[], order: readonly NetworkId[]): Network[] {
  const rank = (id: string) => {
    const i = order.indexOf(id);
    return i < 0 ? Number.MAX_SAFE_INTEGER : i;
  };
  return networks.map((n, i) => ({ n, i })).sort((a, b) => rank(a.n.id) - rank(b.n.id) || a.i - b.i).map((x) => x.n);
}

/** The label to print for a network id this page may not know: the node's own label, else the id as text. */
export const networkLabel = (networks: readonly Network[] | null | undefined, id: string): string => networks?.find((n) => n.id === id)?.label ?? id;

/** `#/messages/net/meshtastic/fc-3`: the colon of `app:deltachat` is percent-encoded. */
export const foreignPath = (network: string, conv?: string): string => `/messages/net/${encodeURIComponent(network)}${conv ? `/${encodeURIComponent(conv)}` : ""}`;

/** A route parameter back to text. A malformed escape is returned as it came. */
export function paramText(raw: string | undefined): string | undefined {
  if (raw === undefined) return undefined;
  try {
    return decodeURIComponent(raw);
  } catch {
    return raw;
  }
}

// --- Conversations ----------------------------------------------------------------------------------------------------

/** Pinned first, then the most recent receipt. */
export const sortConversations = (convs: readonly ForeignConversation[]): ForeignConversation[] =>
  [...convs].sort((a, b) => Number(b.pinned) - Number(a.pinned) || b.last_seq - a.last_seq);

export const channelsOf = (convs: readonly ForeignConversation[]): ForeignConversation[] => sortConversations(convs.filter((c) => c.kind === "channel"));
export const dmsOf = (convs: readonly ForeignConversation[]): ForeignConversation[] => sortConversations(convs.filter((c) => c.kind === "dm"));
/** Recipient lists and app-peer groups. */
export const groupsOf = (convs: readonly ForeignConversation[]): ForeignConversation[] => sortConversations(convs.filter((c) => c.kind === "group"));

/** Replace a conversation list, keeping the previous array when nothing changed. */
export function sameConversations(prev: readonly ForeignConversation[] | undefined, next: readonly ForeignConversation[]): ForeignConversation[] {
  return prev && JSON.stringify(prev) === JSON.stringify(next) ? (prev as ForeignConversation[]) : [...next];
}

// --- Parts of a split send ---------------------------------------------------------------------------------------------

export interface SingleItem {
  kind: "row";
  m: ForeignMessage;
}
/** The rows of one split send, in part order. They are drawn as one item with the lowest state among them. */
export interface PartsItem {
  kind: "parts";
  key: string;
  rows: ForeignMessage[];
}
export type ThreadItem = SingleItem | PartsItem;

/** The key that ties the parts of one split send together: its send group, or for a relayed copy its edge and origin row. */
function partsKey(m: ForeignMessage): string | null {
  if (m.dir !== "own" || !m.part) return null;
  if (m.group_id) return `g:${m.group_id}:${m.conv}`;
  if (m.relayed_from) return `e:${m.relayed_from.edge}:${m.relayed_from.msg}`;
  return null;
}

/** Collapse the parts of each split send into one item, placed where its first part arrived. Everything else stays one row each. */
export function groupParts(messages: readonly ForeignMessage[]): ThreadItem[] {
  const out: ThreadItem[] = [];
  const open = new Map<string, PartsItem>();
  for (const m of messages) {
    const key = partsKey(m);
    if (key === null) {
      out.push({ kind: "row", m });
      continue;
    }
    const item = open.get(key);
    if (item) item.rows.push(m);
    else {
      const created: PartsItem = { kind: "parts", key, rows: [m] };
      open.set(key, created);
      out.push(created);
    }
  }
  for (const item of open.values()) item.rows.sort((a, b) => (a.part?.k ?? 0) - (b.part?.k ?? 0));
  return out;
}

export interface PartsSummary {
  /** The lowest state among the parts. */
  state: string;
  /** Parts the page holds, and the total the send was cut into. */
  have: number;
  total: number;
  /** Parts that can still be dropped (`queued`, nothing sent). Cancel drops these only. */
  unsent: ForeignMessage[];
  /** Parts past `queued` (they are out, and cancel cannot take them back). */
  out: number;
}

export function summarizeParts(rows: readonly ForeignMessage[]): PartsSummary {
  const total = rows.reduce((n, r) => Math.max(n, r.part?.n ?? 0), rows.length);
  return {
    state: lowestState(rows.map((r) => r.state)),
    have: rows.length,
    total,
    unsent: rows.filter(isForeignCancellable),
    out: rows.filter((r) => foreignRank(r.state) >= foreignRank("sent")).length,
  };
}

// --- Table lookups ----------------------------------------------------------------------------------------------------

/** `own()`-guarded lookup, so a value from the node that is not in the table answers undefined, never a function from Object.prototype. */
export function lookup<T>(table: Record<string, T>, key: string): T | undefined {
  return own(table, key) ? table[key] : undefined;
}
