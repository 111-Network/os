// Ignored message requests (os/50-messaging/README.md section 7.2): the user "ignores" a request and it is "kept in the
// request list, never auto-pruned while pending, no ACK sent". The node has no such operation, and calling DELETE on the
// contact would discard the held words and let the sender's next retransmission bring the request back. So ignoring is
// a fold the page keeps for itself, per device: a set of `(peer, newest received_seq)` pairs. The request stays pending
// on the node; this page lists it under "ignored" and leaves it out of the attention count until the peer's request
// gains a message newer than the one the user dismissed.
//
// Pure: no React, no storage of its own (the store hands in a `KeyValueStorage`).
import { own } from "./own";
import type { Contact, NodeId, Thread } from "./types";

/** `peer -> received_seq` of the newest message the request held when the user dismissed it. */
export type IgnoredRequests = Readonly<Record<NodeId, number>>;

export const NO_IGNORED: IgnoredRequests = Object.freeze({});

/** localStorage key of the per-device set. */
export const IGNORED_STORAGE_KEY = "111.ui.ignored-requests";

/** The node holds at most this many pending requests (a pending-request quota, default 256), so the set never needs more. */
export const IGNORED_MAX = 256;

/** The slice of Web Storage this module needs. */
export interface KeyValueStorage {
  getItem(key: string): string | null;
  setItem(key: string, value: string): void;
  removeItem(key: string): void;
}

/** The newest `received_seq` a pending request holds: its last message's. 0 when it holds none. */
export const requestSeq = (t: Pick<Thread, "last">): number => t.last?.received_seq ?? 0;

/**
 * Is this request folded away? Only when the peer is in the set and has nothing newer than what was dismissed. An
 * unknown newest sequence (`null`: the thread is not loaded) never hides a request: not knowing is no reason to hide.
 */
export function isIgnored(ignored: IgnoredRequests, peer: NodeId, seq: number | null): boolean {
  if (seq === null || !own(ignored, peer)) return false;
  return seq <= ignored[peer]!;
}

/** Add (or raise) the entry for `peer`. The set is capped at `IGNORED_MAX`; the oldest entries make room. */
export function ignoreRequest(ignored: IgnoredRequests, peer: NodeId, seq: number): IgnoredRequests {
  const next: Record<NodeId, number> = { ...ignored };
  delete next[peer]; // re-inserted last, so insertion order is recency
  next[peer] = Math.max(seq, own(ignored, peer) ? ignored[peer]! : 0);
  const keys = Object.keys(next);
  for (const k of keys.slice(0, Math.max(0, keys.length - IGNORED_MAX))) delete next[k as NodeId];
  return next;
}

/** Remove the entry for `peer` ("show again"). The same object when there was none. */
export function restoreRequest(ignored: IgnoredRequests, peer: NodeId): IgnoredRequests {
  if (!own(ignored, peer)) return ignored;
  const next: Record<NodeId, number> = { ...ignored };
  delete next[peer];
  return next;
}

/**
 * Drop entries that no longer mean anything: the peer is not a pending request any more (accepted, forgotten), or its
 * request gained a newer message than the one dismissed. Returns the same object when nothing was dropped. A list
 * that is not loaded (`null`) proves nothing, so it drops nothing.
 */
export function pruneIgnored(ignored: IgnoredRequests, contacts: readonly Contact[] | null, threads: readonly Thread[] | null): IgnoredRequests {
  const peers = Object.keys(ignored) as NodeId[];
  if (peers.length === 0) return ignored;
  const pending = contacts ? new Set(contacts.filter((c) => c.request).map((c) => c.node_id)) : null;
  const seqs = threads ? new Map(threads.filter((t) => t.request).map((t) => [t.peer, requestSeq(t)] as const)) : null;
  const keep = peers.filter((p) => {
    if (pending && !pending.has(p)) return false;
    const seq = seqs?.get(p);
    return !(seq !== undefined && seq > ignored[p]!);
  });
  if (keep.length === peers.length) return ignored;
  return Object.fromEntries(keep.map((p) => [p, ignored[p]!]));
}

const HEX64 = /^[0-9a-f]{64}$/;

/** Read the stored set. Anything that is not a well-formed `{ <64 hex>: <non-negative integer> }` is dropped, never trusted. */
export function parseIgnored(raw: string | null): IgnoredRequests {
  if (!raw) return NO_IGNORED;
  let value: unknown;
  try {
    value = JSON.parse(raw);
  } catch {
    return NO_IGNORED;
  }
  if (!value || typeof value !== "object" || Array.isArray(value)) return NO_IGNORED;
  const out: Record<NodeId, number> = {};
  for (const [k, v] of Object.entries(value)) {
    if (HEX64.test(k) && typeof v === "number" && Number.isSafeInteger(v) && v >= 0) out[k] = v;
  }
  const keys = Object.keys(out);
  for (const k of keys.slice(0, Math.max(0, keys.length - IGNORED_MAX))) delete out[k as NodeId];
  return Object.keys(out).length === 0 ? NO_IGNORED : out;
}

/** Read the set from `storage`; empty when there is no storage or it throws (private windows, blocked site data). */
export function loadIgnored(storage: KeyValueStorage | null): IgnoredRequests {
  try {
    return parseIgnored(storage?.getItem(IGNORED_STORAGE_KEY) ?? null);
  } catch {
    return NO_IGNORED;
  }
}

/** Write the set. A page without storage still works; the fold then lasts until reload. */
export function saveIgnored(storage: KeyValueStorage | null, ignored: IgnoredRequests): void {
  try {
    if (!storage) return;
    if (Object.keys(ignored).length === 0) storage.removeItem(IGNORED_STORAGE_KEY);
    else storage.setItem(IGNORED_STORAGE_KEY, JSON.stringify(ignored));
  } catch {
    /* storage can be full or blocked; the fold still holds for this page load */
  }
}

/** localStorage when this browser allows it (the accessor itself can throw), else null. */
export function browserStorage(): KeyValueStorage | null {
  try {
    return typeof localStorage === "undefined" ? null : localStorage;
  } catch {
    return null;
  }
}
