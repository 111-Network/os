// Tiny external store for the live node state. Pure TypeScript (no React); `hooks.ts` wraps it in
// useSyncExternalStore. The store is the only consumer of the adapter's read side and of the
// events stream: components read through hooks and never fetch snapshots themselves.
import type { Adapter, ConnectionInfo, ConnectionState, EventsFrameWithId } from "./adapter";
import { isApiError, type ApiError } from "./problem";
import type { AgentEvent, DeviceRecord, Proposal, ServiceRecord, Status } from "./types";

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
  /** open proposals + services in `error` + devices in `error`; drives the header badge. */
  count: number;
  openProposals: number;
  erroredServices: number;
  erroredDevices: number;
}

export interface StoreMeta {
  /** The first refreshAll() has finished (successfully or not). */
  ready: boolean;
  /** A snapshot call came back 401: the shell should route to /pair. */
  needsPair: boolean;
  /** Last snapshot failure, cleared by the next success. */
  error: ApiError | null;
}

export interface StoreState {
  status: Status | null;
  services: ServiceRecord[] | null;
  /** The device registry; null until loaded. */
  devices: DeviceRecord[] | null;
  /** All proposals (open and decided), newest first. */
  proposals: Proposal[] | null;
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
}

export function isDisconnected(state: ConnectionState, everOpened: boolean): boolean {
  return state === "stale" || state === "closed" || (state === "connecting" && everOpened);
}

/** Pure; also used by the header badge and the Overview "needs you" block. */
export function computeAttention(
  proposals: readonly Proposal[] | null,
  services: readonly ServiceRecord[] | null,
  devices: readonly DeviceRecord[] | null = null,
): Attention {
  const openProposals = proposals ? proposals.filter((p) => p.decision === null).length : 0;
  const erroredServices = services ? services.filter((s) => s.state === "error").length : 0;
  const erroredDevices = devices ? devices.filter((d) => d.state === "error").length : 0;
  return { count: openProposals + erroredServices + erroredDevices, openProposals, erroredServices, erroredDevices };
}

export function openProposalsOf(proposals: readonly Proposal[] | null): Proposal[] {
  return proposals ? proposals.filter((p) => p.decision === null) : [];
}

function sameJson(a: unknown, b: unknown): boolean {
  return a === b || JSON.stringify(a) === JSON.stringify(b);
}

/** Keep object identity for records that did not change, so unrelated components do not re-render. */
function reuse<T>(prev: readonly T[] | null, next: readonly T[]): T[] {
  if (!prev) return [...next];
  return next.map((n) => prev.find((p) => sameJson(p, n)) ?? n);
}

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
    connection: INITIAL_CONNECTION,
    activity: [],
    baselineVersion: null,
    version: null,
    versionChanged: false,
    attention: { count: 0, openProposals: 0, erroredServices: 0, erroredDevices: 0 },
    meta: { ready: false, needsPair: false, error: null },
  };
}

export class ApiStore {
  private state: StoreState = initialState();
  private listeners = new Set<() => void>();
  private adapter: Adapter | null = null;
  private unsubscribe: (() => void) | null = null;
  private readonly now: () => number;

  constructor(deps: { now?: () => number } = {}) {
    this.now = deps.now ?? (() => Date.now());
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
    if ("proposals" in patch || "services" in patch || "devices" in patch) {
      next.attention = computeAttention(next.proposals, next.services, next.devices);
    }
    this.state = next;
    for (const l of [...this.listeners]) l();
  }

  // --- lifecycle ------------------------------------------------------------------------------
  /** Start (or restart) against an adapter: subscribe to the stream and load the snapshots. */
  attach(adapter: Adapter): void {
    this.detach();
    this.adapter = adapter;
    this.unsubscribe = adapter.events({
      onFrame: (f) => this.applyFrame(f),
      onState: (s, info) => this.setConnection(s, info),
    });
    void this.refreshAll();
  }

  detach(): void {
    this.unsubscribe?.();
    this.unsubscribe = null;
    this.adapter = null;
  }

  /** Back to the empty state (tests, and after logout). Listeners stay subscribed. */
  reset(): void {
    this.commit(initialState());
  }

  // --- stream ---------------------------------------------------------------------------------
  setConnection(state: ConnectionState, info?: ConnectionInfo): void {
    const prev = this.state.connection;
    const everOpened = prev.everOpened || state === "open";
    const lastEventAt = info?.lastEventAt ?? prev.lastEventAt;
    if (prev.state === state && prev.lastEventAt === lastEventAt && prev.everOpened === everOpened) return;
    this.commit({ connection: { state, lastEventAt, everOpened, disconnected: isDisconnected(state, everOpened) } });
    // A fatal close is usually a lost session: probe so a 401 reaches the shell.
    if (state === "closed") void this.refreshStatus();
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
        if (frame.data && Array.isArray(frame.data.services)) patch.services = reuse(this.state.services, frame.data.services);
        break;
      case "devices":
        if (frame.data && Array.isArray(frame.data.devices)) patch.devices = reuse(this.state.devices, frame.data.devices);
        break;
      case "proposal":
        refetchProposals = true;
        break;
      case "agent":
        if (frame.data && typeof frame.data.seq === "number") Object.assign(patch, this.activityPatch(frame.data));
        break;
      case "reset":
        refetchAll = true;
        break;
    }
    this.commit(patch);
    if (refetchAll) void this.refreshAll();
    else if (refetchProposals) void this.refreshProposals();
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
    const [st, sv, dv, pr] = await Promise.allSettled([a.status(), a.services(), a.devices(), a.proposals("all")]);
    if (this.adapter !== a) return; // detached or swapped while loading
    const patch: Partial<StoreState> = {};
    if (st.status === "fulfilled") Object.assign(patch, this.statusPatch(st.value));
    if (sv.status === "fulfilled") patch.services = reuse(this.state.services, sv.value);
    if (dv.status === "fulfilled") patch.devices = reuse(this.state.devices, dv.value);
    if (pr.status === "fulfilled") patch.proposals = newestFirst(pr.value);
    const errors = [st, sv, dv, pr].flatMap((r) => (r.status === "rejected" ? [r.reason] : []));
    patch.meta = { ready: true, ...(this.fail(errors) as { needsPair: boolean; error: ApiError | null }) };
    this.commit(patch);
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
  });

  refreshServices = coalesced(async () => {
    const a = this.adapter;
    if (!a) return;
    try {
      const services = await a.services();
      if (this.adapter === a) this.commit({ services: reuse(this.state.services, services) });
    } catch {
      /* keep the previous snapshot; the connection state tells the user */
    }
  });

  refreshDevices = coalesced(async () => {
    const a = this.adapter;
    if (!a) return;
    try {
      const devices = await a.devices();
      if (this.adapter === a) this.commit({ devices: reuse(this.state.devices, devices) });
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
}

/** The app-wide store. Tests construct their own ApiStore. */
export const apiStore = new ApiStore();
