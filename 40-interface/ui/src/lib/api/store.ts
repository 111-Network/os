// Tiny external store for the live node state. Pure TypeScript (no React); `hooks.ts` wraps it in
// useSyncExternalStore. The store is the only consumer of the adapter's read side and of the
// events stream: components read through hooks and never fetch snapshots themselves.
import type { Adapter, ConnectionInfo, ConnectionState, EventsFrameWithId } from "./adapter";
import { openRequests, removeContact, upsertContact } from "./contacts";
import { applyMessageToPage, mergeThreadPage, THREAD_PAGE_LIMIT, trimPage, type ThreadPage } from "./messages";
import { sameSources } from "./nearby";
import { isApiError, type ApiError } from "./problem";
import { browserStorage, ignoreRequest, isIgnored, loadIgnored, NO_IGNORED, pruneIgnored, requestSeq, restoreRequest, saveIgnored, type IgnoredRequests, type KeyValueStorage } from "./requests";
import type { AgentEvent, Card, Contact, ContactChangeEvent, DeviceRecord, FabricStatus, HostedPanel, MessageChangeEvent, NearbySource, NodeId, Proposal, ServiceRecord, Status, Thread } from "./types";

export const ACTIVITY_RING = 200;

export interface ConnectionSnapshot {
  state: ConnectionState;
  /** Epoch ms of the last frame, or null before the first sign of life. */
  lastEventAt: number | null;
  everOpened: boolean;
  /** True when the UI should show the "Disconnected" banner and hollow dots. */
  disconnected: boolean;
}

export interface Attention {
  /** open proposals + services in `error` + devices in `error` + open message requests; drives the header badge. */
  count: number;
  openProposals: number;
  erroredServices: number;
  erroredDevices: number;
  /** Pending message requests (INTROs, Layer 5 §7.2): contacts with `request: true`, less the ones ignored on this device. */
  openRequests: number;
}

export interface StoreMeta {
  /** The first refreshAll() has finished (successfully or not). */
  ready: boolean;
  /** A snapshot call came back 401: the shell should route to /pair. */
  needsPair: boolean;
  /** Last snapshot failure, cleared by the next success. */
  error: ApiError | null;
}

/** One conversation as the store holds it: loading, ready (`page`), or why it could not be loaded. */
export interface ThreadSlot {
  state: "loading" | "ready" | "missing" | "error";
  page: ThreadPage | null;
  error: ApiError | null;
}

export interface StoreState {
  status: Status | null;
  services: ServiceRecord[] | null;
  /** The device registry; null until loaded. */
  devices: DeviceRecord[] | null;
  /** All proposals (open and decided), newest first. */
  proposals: Proposal[] | null;
  /** Layer 5: conversations, most recent activity first (message requests included); null until loaded. */
  threads: Thread[] | null;
  /** Layer 5: conversations opened this page load, by peer NodeId. */
  threadPages: Record<NodeId, ThreadSlot>;
  /** Layer 5: contacts, pending message requests included (`request: true`); null until loaded. */
  contacts: Contact[] | null;
  /**
   * Layer 5: requests the user ignored on this device, `peer -> newest received_seq` at the time. The request stays
   * pending on the node (§7.2: kept, never auto-pruned, no ACK); this is only a fold, cleared by newer activity.
   */
  ignored: IgnoredRequests;
  /** Layer 5: this node's own card, loaded on first use. */
  ownCard: Card | null;
  /** Layer 5: links, home hub, custody, store, settings; null until loaded. */
  fabric: FabricStatus | null;
  /** Layer 5: presence per source; null until loaded. Names are untrusted RF-origin strings. */
  nearby: NearbySource[] | null;
  /** Layer 5: declared hosted panels and their listener slots; null until loaded. */
  panels: HostedPanel[] | null;
  connection: ConnectionSnapshot;
  /** Agent events, newest first, at most ACTIVITY_RING, deduped on (seq, ts). */
  activity: AgentEvent[];
  /** `status.node.version` as of the first status this page load saw. */
  baselineVersion: string | null;
  /** Current `status.node.version`. */
  version: string | null;
  /** The node updated underneath this page: show "Node updated - reload". */
  versionChanged: boolean;
  attention: Attention;
  meta: StoreMeta;
  /**
   * Bumped every time a fresh service list arrives (a `services` frame or a snapshot). A control that
   * acted on a service (restart) waits for this to move before it stops showing "restarting".
   */
  servicesRev: number;
}

export function isDisconnected(state: ConnectionState, everOpened: boolean): boolean {
  return state === "stale" || state === "closed" || (state === "connecting" && everOpened);
}

/**
 * Pure; also used by the header badge and the Overview "needs you" block. A request the user ignored on this device
 * (`ignored`, judged against its thread's newest receipt in `threads`) is still pending but no longer needs them.
 */
export function computeAttention(
  proposals: readonly Proposal[] | null,
  services: readonly ServiceRecord[] | null,
  devices: readonly DeviceRecord[] | null = null,
  contacts: readonly Contact[] | null = null,
  ignored: IgnoredRequests = NO_IGNORED,
  threads: readonly Thread[] | null = null,
): Attention {
  const openProposals = proposals ? proposals.filter((p) => p.decision === null).length : 0;
  const erroredServices = services ? services.filter((s) => s.state === "error").length : 0;
  const erroredDevices = devices ? devices.filter((d) => d.state === "error").length : 0;
  let pending = openRequests(contacts);
  if (pending.length > 0 && ignored !== NO_IGNORED && Object.keys(ignored).length > 0) {
    const seqs = new Map((threads ?? []).filter((t) => t.request).map((t) => [t.peer, requestSeq(t)] as const));
    pending = pending.filter((c) => !isIgnored(ignored, c.node_id, seqs.get(c.node_id) ?? null));
  }
  const requests = pending.length;
  return { count: openProposals + erroredServices + erroredDevices + requests, openProposals, erroredServices, erroredDevices, openRequests: requests };
}

export function openProposalsOf(proposals: readonly Proposal[] | null): Proposal[] {
  return proposals ? proposals.filter((p) => p.decision === null) : [];
}

function sameJson(a: unknown, b: unknown): boolean {
  return a === b || JSON.stringify(a) === JSON.stringify(b);
}

/**
 * Keep object identity for records that did not change, so unrelated components do not re-render. Records are matched
 * by `key` (their id), so a reload costs one comparison per record, not one per pair: a list of hundreds of threads
 * stays cheap on a phone.
 */
function reuse<T>(prev: readonly T[] | null, next: readonly T[], key: (item: T) => string): T[] {
  if (!prev) return [...next];
  const before = new Map<string, T>();
  for (const p of prev) before.set(key(p), p);
  return next.map((n) => {
    const p = before.get(key(n));
    return p !== undefined && sameJson(p, n) ? p : n;
  });
}

const byId = (x: { id: string }): string => x.id;
const byPeer = (t: Thread): string => t.peer;
const byNodeId = (c: Contact): string => c.node_id;

function coalesced(fn: () => Promise<void>): () => Promise<void> {
  let running: Promise<void> | null = null;
  let again = false;
  return () => {
    if (running) {
      again = true; // run once more after the current pass, whatever the number of callers
      return running;
    }
    running = (async () => {
      do {
        again = false;
        await fn();
      } while (again);
    })().finally(() => {
      running = null;
    });
    return running;
  };
}

function newestFirst(list: readonly Proposal[]): Proposal[] {
  return [...list].sort((a, b) => (a.created_at < b.created_at ? 1 : a.created_at > b.created_at ? -1 : 0));
}

const INITIAL_CONNECTION: ConnectionSnapshot = { state: "connecting", lastEventAt: null, everOpened: false, disconnected: false };

function initialState(): StoreState {
  return {
    status: null,
    services: null,
    devices: null,
    proposals: null,
    threads: null,
    threadPages: {},
    contacts: null,
    ignored: NO_IGNORED,
    ownCard: null,
    fabric: null,
    nearby: null,
    panels: null,
    connection: INITIAL_CONNECTION,
    activity: [],
    baselineVersion: null,
    version: null,
    versionChanged: false,
    attention: { count: 0, openProposals: 0, erroredServices: 0, erroredDevices: 0, openRequests: 0 },
    meta: { ready: false, needsPair: false, error: null },
    servicesRev: 0,
  };
}

export class ApiStore {
  private state: StoreState = initialState();
  private listeners = new Set<() => void>();
  private adapter: Adapter | null = null;
  private unsubscribe: (() => void) | null = null;
  /** The snapshot load started by the latest `attach`, while it is still running. */
  private initialLoad: Promise<void> | null = null;
  private readonly now: () => number;
  /** The `limit` each loaded conversation was last fetched with, so a reload keeps the same window. */
  private readonly pageLimits = new Map<NodeId, number>();
  /** The newest `loadThread` call per conversation; an older response that arrives later is dropped. */
  private readonly threadGen = new Map<NodeId, number>();
  /** Never repeats (not even across `reset`), so a response from before a reset cannot pass for a current one. */
  private loadCounter = 0;
  /** Where the per-device fold of ignored requests is kept; null when the browser gives none. */
  private readonly storage: KeyValueStorage | null;

  constructor(deps: { now?: () => number; storage?: KeyValueStorage | null } = {}) {
    this.now = deps.now ?? (() => Date.now());
    this.storage = deps.storage === undefined ? browserStorage() : deps.storage;
    this.state = { ...this.state, ignored: loadIgnored(this.storage) };
  }

  // --- useSyncExternalStore surface (arrow functions: safe to pass unbound) -------------------
  getState = (): StoreState => this.state;
  subscribe = (listener: () => void): (() => void) => {
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
    };
  };

  private commit(patch: Partial<StoreState>): void {
    const next: StoreState = { ...this.state, ...patch };
    if ("contacts" in patch || "threads" in patch) {
      // An ignored request that was accepted, forgotten, or wrote again is no longer folded.
      const ignored = pruneIgnored(next.ignored, next.contacts, next.threads);
      if (ignored !== next.ignored) {
        next.ignored = ignored;
        saveIgnored(this.storage, ignored);
      }
    }
    if ("proposals" in patch || "services" in patch || "devices" in patch || "contacts" in patch || "threads" in patch || "ignored" in patch) {
      next.attention = computeAttention(next.proposals, next.services, next.devices, next.contacts, next.ignored, next.threads);
    }
    this.state = next;
    for (const l of [...this.listeners]) l();
  }

  // --- ignored message requests (a per-device fold; the node is never asked, §7.2) ---------------
  /**
   * Fold a pending request away on this device: it stays pending on the node (no ACK, nothing sent, nothing deleted),
   * is listed under "ignored", and leaves the attention count until the peer writes again. `seq` is the newest
   * `received_seq` the request holds now.
   */
  ignoreRequest(peer: NodeId, seq: number): void {
    const ignored = ignoreRequest(this.state.ignored, peer, seq);
    saveIgnored(this.storage, ignored);
    this.commit({ ignored });
  }

  /** Show an ignored request again. */
  restoreRequest(peer: NodeId): void {
    const ignored = restoreRequest(this.state.ignored, peer);
    if (ignored === this.state.ignored) return;
    saveIgnored(this.storage, ignored);
    this.commit({ ignored });
  }

  // --- lifecycle ------------------------------------------------------------------------------
  /**
   * Start (or restart) against an adapter: subscribe to the stream and load the snapshots. The returned
   * promise settles when that first load has finished (it never rejects).
   */
  attach(adapter: Adapter): Promise<void> {
    this.detach();
    this.adapter = adapter;
    this.subscribeStream();
    // Recorded before the stream can deliver anything: the daemon's mandatory first `reset` frame must
    // not trigger a second load of what this one is already fetching.
    const load = this.refreshAll();
    this.initialLoad = load;
    void load.finally(() => {
      if (this.initialLoad === load) this.initialLoad = null;
    });
    return load;
  }

  detach(): void {
    this.unsubscribe?.();
    this.unsubscribe = null;
    this.adapter = null;
    this.initialLoad = null;
  }

  private subscribeStream(): void {
    const a = this.adapter;
    if (!a || this.unsubscribe) return;
    this.unsubscribe = a.events({
      onFrame: (f) => this.applyFrame(f),
      onState: (s, info) => this.setConnection(s, info),
    });
  }

  /**
   * An unpaired visitor gets 401 from /events and the browser would retry it forever: stop the stream
   * while `meta.needsPair` is set and start it again once the node accepts us. Runs after every snapshot.
   */
  private syncStream(): void {
    if (!this.adapter) return;
    if (this.state.meta.needsPair) {
      if (!this.unsubscribe) return;
      this.unsubscribe();
      this.unsubscribe = null;
      // The watchdog died with the stream: an "open" that nobody is watching any more is a lie.
      const c = this.state.connection;
      if (c.state === "open") this.commit({ connection: { ...c, state: "closed", disconnected: true } });
    } else {
      this.subscribeStream();
    }
  }

  /** Back to the empty state (tests, and after logout). Listeners stay subscribed. */
  reset(): void {
    this.pageLimits.clear();
    this.threadGen.clear();
    this.commit({ ...initialState(), ignored: loadIgnored(this.storage) });
  }

  // --- stream ---------------------------------------------------------------------------------
  setConnection(state: ConnectionState, info?: ConnectionInfo): void {
    const prev = this.state.connection;
    const everOpened = prev.everOpened || state === "open";
    const lastEventAt = info?.lastEventAt ?? prev.lastEventAt;
    if (prev.state === state && prev.lastEventAt === lastEventAt && prev.everOpened === everOpened) return;
    this.commit({ connection: { state, lastEventAt, everOpened, disconnected: isDisconnected(state, everOpened) } });
    // A fatal close is usually a lost session: probe so a 401 reaches the shell.
    if (state === "closed" && this.unsubscribe) void this.refreshStatus();
  }

  applyFrame(frame: EventsFrameWithId): void {
    const patch: Partial<StoreState> = {};
    // Any frame is proof of life.
    const c = this.state.connection;
    const at = this.now();
    if (c.lastEventAt !== at || c.state !== "open") {
      patch.connection = { state: "open", lastEventAt: at, everOpened: true, disconnected: false };
    }
    let refetchProposals = false;
    let refetchAll = false;
    let refetchThreads = false;
    switch (frame.event) {
      case "status": {
        // The stream is shared across subscribers, so a frame's `auth` block cannot be per-viewer:
        // keep the `auth` we learned from our own GET /status (it only changes on pair/logout).
        if (frame.data && typeof frame.data === "object" && frame.data.node) {
          const auth = this.state.status?.auth ?? frame.data.auth;
          Object.assign(patch, this.statusPatch({ ...frame.data, auth }));
        }
        break;
      }
      case "services":
        if (frame.data && Array.isArray(frame.data.services)) {
          patch.services = reuse(this.state.services, frame.data.services, byId);
          patch.servicesRev = this.state.servicesRev + 1;
        }
        break;
      case "devices":
        if (frame.data && Array.isArray(frame.data.devices)) patch.devices = reuse(this.state.devices, frame.data.devices, byId);
        break;
      case "proposal":
        refetchProposals = true;
        break;
      case "agent":
        if (frame.data && typeof frame.data.seq === "number") Object.assign(patch, this.activityPatch(frame.data));
        break;
      case "message":
        // The conversation, if it is open, takes the message at once; the list (last line, unread, state counts) is
        // cheaper to reload than to patch, and a coalesced reload is one request however many frames arrive.
        if (isMessageFrame(frame.data)) {
          Object.assign(patch, this.messagePatch(frame.data));
          refetchThreads = true;
        }
        break;
      case "contact":
        if (isContactFrame(frame.data)) {
          Object.assign(patch, this.contactPatch(frame.data));
          refetchThreads = true; // alias, accepted requests and forgotten peers all change the thread list
        }
        break;
      case "fabric":
        if (frame.data && Array.isArray(frame.data.links)) patch.fabric = sameJson(this.state.fabric, frame.data) ? this.state.fabric : frame.data;
        break;
      case "nearby":
        if (frame.data && Array.isArray(frame.data.sources)) patch.nearby = sameSources(this.state.nearby, frame.data.sources);
        break;
      case "panels":
        if (frame.data && Array.isArray(frame.data.panels)) patch.panels = reuse(this.state.panels, frame.data.panels, byId);
        break;
      case "reset":
        // The first reset after attach is the daemon saying "start from a snapshot". That snapshot is
        // already on its way; only a reset after the initial load is a reason to load again.
        refetchAll = this.initialLoad === null;
        break;
    }
    this.commit(patch);
    if (refetchAll) void this.refreshAll();
    else {
      if (refetchProposals) void this.refreshProposals();
      if (refetchThreads) void this.refreshThreads();
    }
  }

  private messagePatch(ev: MessageChangeEvent): Partial<StoreState> {
    const slot = this.state.threadPages[ev.peer];
    if (!slot?.page) return {};
    const page = applyMessageToPage(slot.page, ev.peer, ev.message, ev.change, this.pageLimits.get(ev.peer) ?? THREAD_PAGE_LIMIT);
    if (!page || page === slot.page) return {};
    return { threadPages: { ...this.state.threadPages, [ev.peer]: { ...slot, page } } };
  }

  private contactPatch(ev: ContactChangeEvent): Partial<StoreState> {
    const contacts = ev.change === "removed" ? removeContact(this.state.contacts, ev.contact.node_id) : upsertContact(this.state.contacts, ev.contact);
    const patch: Partial<StoreState> = { contacts };
    // A request that was accepted or a peer that changed: an open conversation follows the flag.
    const slot = this.state.threadPages[ev.contact.node_id];
    if (slot?.page && slot.page.request !== ev.contact.request && ev.change !== "removed") {
      patch.threadPages = { ...this.state.threadPages, [ev.contact.node_id]: { ...slot, page: { ...slot.page, request: ev.contact.request } } };
    }
    return patch;
  }

  private statusPatch(status: Status): Partial<StoreState> {
    if (sameJson(this.state.status, status)) return {};
    const version = status.node.version ?? null;
    const baselineVersion = this.state.baselineVersion ?? version;
    return {
      status,
      version,
      baselineVersion,
      versionChanged: baselineVersion !== null && version !== null && version !== baselineVersion,
    };
  }

  private activityPatch(ev: AgentEvent): Partial<StoreState> {
    if (this.state.activity.some((a) => a.seq === ev.seq && a.ts === ev.ts)) return {};
    return { activity: [ev, ...this.state.activity].slice(0, ACTIVITY_RING) };
  }

  // --- snapshots ------------------------------------------------------------------------------
  private fail(errors: unknown[]): Partial<StoreMeta> {
    const apiErrors = errors.filter(isApiError);
    return { needsPair: apiErrors.some((e) => e.unauthenticated), error: apiErrors[0] ?? null };
  }

  /** Reload status, services, devices and proposals. Concurrent calls coalesce into one trailing pass. */
  refreshAll = coalesced(async () => {
    const a = this.adapter;
    if (!a) return;
    // Layer 5 reads are optional: an older daemon answers 404, and an adapter without them simply leaves the slice empty.
    const optional = <T,>(read: (x: Adapter) => Promise<T>): Promise<T> => {
      try {
        return read(a);
      } catch (e) {
        return Promise.reject(e);
      }
    };
    const [st, sv, dv, pr, th, ct, fb, nb, pn] = await Promise.allSettled([
      a.status(),
      a.services(),
      a.devices(),
      a.proposals("all"),
      optional((x) => x.messages.threads()),
      optional((x) => x.contacts.list()),
      optional((x) => x.fabric.status()),
      optional((x) => x.nearby()),
      optional((x) => x.panels.list()),
    ]);
    if (this.adapter !== a) return; // detached or swapped while loading
    const patch: Partial<StoreState> = {};
    if (st.status === "fulfilled") Object.assign(patch, this.statusPatch(st.value));
    if (sv.status === "fulfilled") {
      patch.services = reuse(this.state.services, sv.value, byId);
      patch.servicesRev = this.state.servicesRev + 1;
    }
    if (dv.status === "fulfilled") patch.devices = reuse(this.state.devices, dv.value, byId);
    if (pr.status === "fulfilled") patch.proposals = newestFirst(pr.value);
    if (th.status === "fulfilled") patch.threads = reuse(this.state.threads, th.value, byPeer);
    if (ct.status === "fulfilled") patch.contacts = reuse(this.state.contacts, ct.value, byNodeId);
    if (fb.status === "fulfilled") patch.fabric = sameJson(this.state.fabric, fb.value) ? this.state.fabric : fb.value;
    if (nb.status === "fulfilled") patch.nearby = sameSources(this.state.nearby, nb.value);
    if (pn.status === "fulfilled") patch.panels = reuse(this.state.panels, pn.value, byId);
    const errors = [st, sv, dv, pr].flatMap((r) => (r.status === "rejected" ? [r.reason] : []));
    // A Layer 5 read can only add the pairing signal: its other failures keep the previous slice and stay quiet.
    for (const r of [th, ct, fb, nb, pn]) if (r.status === "rejected" && isApiError(r.reason) && r.reason.unauthenticated) errors.push(r.reason);
    patch.meta = { ready: true, ...(this.fail(errors) as { needsPair: boolean; error: ApiError | null }) };
    this.commit(patch);
    this.syncStream();
    // Conversations that are open follow a reload, with the window they had.
    for (const peer of Object.keys(this.state.threadPages)) void this.loadThread(peer, this.pageLimits.get(peer));
  });

  refreshStatus = coalesced(async () => {
    const a = this.adapter;
    if (!a) return;
    const patch: Partial<StoreState> = {};
    let err: unknown = null;
    try {
      Object.assign(patch, this.statusPatch(await a.status()));
    } catch (e) {
      err = e;
    }
    if (this.adapter !== a) return;
    patch.meta = { ...this.state.meta, ...(err ? this.fail([err]) : { needsPair: false, error: null }) } as StoreMeta;
    this.commit(patch);
    this.syncStream();
  });

  refreshServices = coalesced(async () => {
    const a = this.adapter;
    if (!a) return;
    try {
      const services = await a.services();
      if (this.adapter === a) this.commit({ services: reuse(this.state.services, services, byId), servicesRev: this.state.servicesRev + 1 });
    } catch {
      /* keep the previous snapshot; the connection state tells the user */
    }
  });

  refreshDevices = coalesced(async () => {
    const a = this.adapter;
    if (!a) return;
    try {
      const devices = await a.devices();
      if (this.adapter === a) this.commit({ devices: reuse(this.state.devices, devices, byId) });
    } catch {
      /* keep the previous snapshot */
    }
  });

  refreshProposals = coalesced(async () => {
    const a = this.adapter;
    if (!a) return;
    try {
      const proposals = await a.proposals("all");
      if (this.adapter === a) this.commit({ proposals: newestFirst(proposals) });
    } catch {
      /* keep the previous snapshot */
    }
  });

  // --- Layer 5 snapshots (messages, contacts, fabric, nearby, panels) ---------------------------------

  refreshThreads = coalesced(async () => {
    const a = this.adapter;
    if (!a) return;
    try {
      const threads = await a.messages.threads();
      if (this.adapter === a) this.commit({ threads: reuse(this.state.threads, threads, byPeer) });
    } catch {
      /* keep the previous list */
    }
  });

  refreshContacts = coalesced(async () => {
    const a = this.adapter;
    if (!a) return;
    try {
      const contacts = await a.contacts.list();
      if (this.adapter === a) this.commit({ contacts: reuse(this.state.contacts, contacts, byNodeId) });
    } catch {
      /* keep the previous list */
    }
  });

  refreshFabric = coalesced(async () => {
    const a = this.adapter;
    if (!a) return;
    try {
      const fabric = await a.fabric.status();
      if (this.adapter === a) this.commit({ fabric: sameJson(this.state.fabric, fabric) ? this.state.fabric : fabric });
    } catch {
      /* keep the previous snapshot */
    }
  });

  refreshNearby = coalesced(async () => {
    const a = this.adapter;
    if (!a) return;
    try {
      const sources = await a.nearby();
      if (this.adapter === a) this.commit({ nearby: sameSources(this.state.nearby, sources) });
    } catch {
      /* keep the previous snapshot */
    }
  });

  refreshPanels = coalesced(async () => {
    const a = this.adapter;
    if (!a) return;
    try {
      const panels = await a.panels.list();
      if (this.adapter === a) this.commit({ panels: reuse(this.state.panels, panels, byId) });
    } catch {
      /* keep the previous snapshot */
    }
  });

  /**
   * Load one conversation (the newest `limit` messages, default 100). While it loads the slot is `loading` (or keeps the
   * page it already had); a 404 reads `missing`. The result replaces the slot only when something changed. Calls for
   * the same conversation overlap freely: only the newest call's answer is taken, and an answer is laid over what a
   * `message` frame has already added rather than over it (see `mergeThreadPage`).
   */
  async loadThread(peer: NodeId, limit: number = THREAD_PAGE_LIMIT): Promise<void> {
    const a = this.adapter;
    if (!a) return;
    this.pageLimits.set(peer, limit);
    const gen = ++this.loadCounter;
    this.threadGen.set(peer, gen);
    const have = this.state.threadPages[peer];
    if (!have) this.commit({ threadPages: { ...this.state.threadPages, [peer]: { state: "loading", page: null, error: null } } });
    try {
      const fetched = await a.messages.thread(peer, { limit });
      if (this.adapter !== a || this.threadGen.get(peer) !== gen) return; // detached, or a newer call is in flight
      const prev = this.state.threadPages[peer];
      const page = trimPage(mergeThreadPage(prev?.page, fetched), this.pageLimits.get(peer) ?? limit);
      const slot: ThreadSlot = prev?.page && sameJson(prev.page, page) ? { ...prev, state: "ready", error: null } : { state: "ready", page, error: null };
      this.commit({ threadPages: { ...this.state.threadPages, [peer]: slot } });
    } catch (e) {
      if (this.adapter !== a || this.threadGen.get(peer) !== gen) return;
      const error = isApiError(e) ? e : null;
      const prev = this.state.threadPages[peer];
      const next: ThreadSlot = error?.status === 404 ? { state: "missing", page: null, error } : { state: prev?.page ? "ready" : "error", page: prev?.page ?? null, error };
      this.commit({ threadPages: { ...this.state.threadPages, [peer]: next } });
    }
  }

  /** Load this node's own card once (public material; it only changes when the identity does). */
  loadOwnCard = coalesced(async () => {
    const a = this.adapter;
    if (!a || this.state.ownCard) return;
    try {
      const card = await a.contacts.ownCard();
      if (this.adapter === a) this.commit({ ownCard: card });
    } catch {
      /* the view says the card could not be loaded; asking again is a remount */
    }
  });
}

function isMessageFrame(d: unknown): d is MessageChangeEvent {
  const x = d as Partial<MessageChangeEvent> | null;
  return !!x && typeof x === "object" && typeof x.peer === "string" && !!x.message && typeof x.message === "object" && typeof x.message.id === "string";
}

function isContactFrame(d: unknown): d is ContactChangeEvent {
  const x = d as Partial<ContactChangeEvent> | null;
  return !!x && typeof x === "object" && !!x.contact && typeof x.contact === "object" && typeof x.contact.node_id === "string";
}

/** The app-wide store. Tests construct their own ApiStore. */
export const apiStore = new ApiStore();
