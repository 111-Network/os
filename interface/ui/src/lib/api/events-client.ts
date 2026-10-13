// Client for the one live stream, GET /api/v1/events, built on EventSource (cookies ride along
// same-origin; the browser reconnects by itself and sends Last-Event-ID). Everything it needs from
// the environment is injected, so it runs under vitest with a fake EventSource and a fake clock.
//
// What this adds on top of EventSource:
//  - named listeners for each event in the contract (EVENTS_EVENT_NAMES), parsed to typed frames;
//  - a staleness watchdog (a half-open connection never fires `error`);
//  - a manual retry with backoff when the browser gives up (readyState CLOSED, e.g. HTTP 401).
//    A fresh EventSource cannot send Last-Event-ID, so the daemon answers with `reset` and the
//    store refetches the snapshots, as the contract prescribes.
import type { ConnectionState, EventsHandlers } from "./adapter";
import { EVENTS_EVENT_NAMES, type EventsFrame } from "./types";

export interface EventSourceLike {
  readonly readyState: number;
  addEventListener(type: string, listener: (ev: { data?: unknown; lastEventId?: string }) => void): void;
  close(): void;
}
export type EventSourceCtor = new (url: string, init?: { withCredentials?: boolean }) => EventSourceLike;

export interface EventsClientDeps {
  EventSource?: EventSourceCtor;
  url: string;
  now: () => number;
  setTimer: (fn: () => void, ms: number) => unknown;
  clearTimer: (handle: unknown) => void;
  /** Silence longer than this marks the stream stale. See DEFAULT_STALE_AFTER_MS. */
  staleAfterMs?: number;
  retryMinMs?: number;
  retryMaxMs?: number;
}

/**
 * The spec says 20 s of silence, but EventSource cannot see the `: keep-alive` comments (every
 * 15 s) and the only visible heartbeat is `status` every 30 s. 20 s would flap "Disconnected"
 * on a healthy idle connection, so the default is the heartbeat plus a 15 s margin.
 * If the daemon ever heartbeats faster than 15 s, lower this to 20 000.
 */
export const DEFAULT_STALE_AFTER_MS = 45_000;

const READY_CLOSED = 2;

export function connectEvents(deps: EventsClientDeps, handlers: EventsHandlers): () => void {
  const staleAfterMs = deps.staleAfterMs ?? DEFAULT_STALE_AFTER_MS;
  const retryMin = deps.retryMinMs ?? 1000;
  const retryMax = deps.retryMaxMs ?? 15_000;

  let disposed = false;
  let current: EventSourceLike | null = null;
  let state: ConnectionState | null = null;
  let lastEventAt: number | undefined;
  let lastEventId: string | undefined;
  let staleTimer: unknown = null;
  let retryTimer: unknown = null;
  let attempt = 0;

  const info = () => ({ lastEventAt, lastEventId });

  function setState(next: ConnectionState): void {
    if (disposed || state === next) return;
    state = next;
    handlers.onState(next, info());
  }

  function clearStale(): void {
    if (staleTimer !== null) deps.clearTimer(staleTimer);
    staleTimer = null;
  }

  function onStaleTimer(): void {
    staleTimer = null;
    if (disposed || !current || lastEventAt === undefined) return;
    const idle = deps.now() - lastEventAt;
    if (idle < staleAfterMs) {
      staleTimer = deps.setTimer(onStaleTimer, staleAfterMs - idle);
      return;
    }
    if (state === "open") setState("stale");
  }

  function armStale(): void {
    clearStale();
    staleTimer = deps.setTimer(onStaleTimer, staleAfterMs);
  }

  function scheduleRetry(): void {
    const delay = Math.min(retryMax, retryMin * 2 ** attempt);
    attempt++;
    retryTimer = deps.setTimer(() => {
      retryTimer = null;
      if (!disposed) open();
    }, delay);
  }

  function onMessage(es: EventSourceLike, name: EventsFrame["event"], ev: { data?: unknown; lastEventId?: string }): void {
    if (disposed || current !== es) return;
    lastEventAt = deps.now();
    if (ev.lastEventId) lastEventId = ev.lastEventId;
    armStale();
    if (state !== "open") setState("open");
    let data: unknown;
    try {
      data = JSON.parse(String(ev.data));
    } catch {
      return; // malformed payload: counts as liveness, but is not delivered
    }
    if (typeof data !== "object" || data === null) return;
    const frame = { event: name, data } as EventsFrame;
    handlers.onFrame(ev.lastEventId ? { ...frame, id: ev.lastEventId } : frame);
  }

  function open(): void {
    const ES = deps.EventSource;
    if (!ES) {
      setState("closed");
      return;
    }
    setState("connecting");
    let es: EventSourceLike;
    try {
      es = new ES(deps.url);
    } catch {
      setState("closed");
      scheduleRetry();
      return;
    }
    current = es;
    es.addEventListener("open", () => {
      if (disposed || current !== es) return;
      attempt = 0;
      lastEventAt = deps.now();
      armStale();
      setState("open");
    });
    es.addEventListener("error", () => {
      if (disposed || current !== es) return;
      clearStale();
      if (es.readyState === READY_CLOSED) {
        es.close();
        current = null;
        setState("closed");
        scheduleRetry();
      } else {
        setState("connecting"); // the browser is retrying and will send Last-Event-ID
      }
    });
    for (const name of EVENTS_EVENT_NAMES) es.addEventListener(name, (ev) => onMessage(es, name, ev));
  }

  open();

  return () => {
    if (disposed) return;
    disposed = true;
    clearStale();
    if (retryTimer !== null) deps.clearTimer(retryTimer);
    retryTimer = null;
    current?.close();
    current = null;
  };
}
