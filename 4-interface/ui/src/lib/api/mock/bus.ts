// MockBus: stands in for 111d's /api/v1/events broadcaster. Same semantics as the real stream:
// one global monotonic `id`, a ring of 256, `Last-Event-ID` replay, otherwise a `reset` frame;
// `status` is re-sent every 30 s as a heartbeat. Also models the browser's automatic reconnect
// (disconnect(forMs)) so banners and the replay path can be exercised without a daemon.
import type { ConnectionState, EventsFrameWithId, EventsHandlers } from "../adapter";
import type { EventsFrame, Status } from "../types";

export interface MockClock {
  now(): number;
  setTimer(fn: () => void, ms: number): unknown;
  clearTimer(handle: unknown): void;
}

export const realClock: MockClock = {
  now: () => Date.now(),
  setTimer: (fn, ms) => setTimeout(fn, ms),
  clearTimer: (h) => clearTimeout(h as ReturnType<typeof setTimeout>),
};

export const RING_CAPACITY = 256;
export const HEARTBEAT_MS = 30_000;

interface Entry {
  id: number;
  frame: EventsFrame;
}

interface Sub {
  handlers: EventsHandlers;
  /** Frames are delivered only while online. */
  online: boolean;
  disposed: boolean;
  lastId: number | undefined;
  lifetime: number;
  /** Waiting for the gate (e.g. unpaired) to open. */
  gated: boolean;
}

export interface BusOptions {
  clock: MockClock;
  snapshot: () => Status;
  capacity?: number;
  heartbeatMs?: number;
  /** When it returns false a new subscription is refused like a 401: state `closed`, no frames. */
  gate?: () => boolean;
}

export class MockBus {
  private ring: Entry[] = [];
  private nextId = 1;
  private lifetime = 1;
  private subs = new Set<Sub>();
  private heartbeat: unknown = null;
  private readonly timers = new Set<unknown>();
  private readonly capacity: number;
  private readonly heartbeatMs: number;

  constructor(private readonly opts: BusOptions) {
    this.capacity = opts.capacity ?? RING_CAPACITY;
    this.heartbeatMs = opts.heartbeatMs ?? HEARTBEAT_MS;
  }

  /** Begin the 30 s status heartbeat. */
  start(): void {
    if (this.heartbeat !== null) return;
    const tick = () => {
      this.publish({ event: "status", data: this.opts.snapshot() });
      this.heartbeat = this.opts.clock.setTimer(tick, this.heartbeatMs);
    };
    this.heartbeat = this.opts.clock.setTimer(tick, this.heartbeatMs);
  }

  stop(): void {
    if (this.heartbeat !== null) this.opts.clock.clearTimer(this.heartbeat);
    this.heartbeat = null;
    for (const t of this.timers) this.opts.clock.clearTimer(t);
    this.timers.clear();
  }

  /** Append to the ring and deliver to every online subscriber. Returns the assigned id. */
  publish(frame: EventsFrame): number {
    const id = this.nextId++;
    this.ring.push({ id, frame });
    if (this.ring.length > this.capacity) this.ring.splice(0, this.ring.length - this.capacity);
    for (const s of this.subs) {
      if (s.online && !s.disposed) this.deliver(s, { id, frame });
    }
    return id;
  }

  private deliver(s: Sub, e: Entry): void {
    s.lastId = e.id;
    s.lifetime = this.lifetime;
    s.handlers.onFrame({ ...e.frame, id: String(e.id) } as EventsFrameWithId);
  }

  /**
   * Open a stream. `lastEventId` plays the part of the Last-Event-ID header: frames after it are
   * replayed when still in the ring, otherwise a `reset` frame comes first.
   */
  subscribe(handlers: EventsHandlers, lastEventId?: number): () => void {
    const sub: Sub = { handlers, online: false, disposed: false, lastId: lastEventId, lifetime: this.lifetime, gated: false };
    this.subs.add(sub);
    this.connect(sub, lastEventId);
    return () => {
      sub.disposed = true;
      this.subs.delete(sub);
    };
  }

  private state(s: Sub, st: ConnectionState): void {
    if (!s.disposed) s.handlers.onState(st, { lastEventAt: this.opts.clock.now() });
  }

  private connect(s: Sub, lastEventId: number | undefined): void {
    this.state(s, "connecting");
    if (this.opts.gate && !this.opts.gate()) {
      s.gated = true;
      queueMicrotask(() => this.state(s, "closed"));
      return;
    }
    s.gated = false;
    queueMicrotask(() => {
      if (s.disposed) return;
      s.online = true;
      this.state(s, "open");
      this.catchUp(s, lastEventId);
    });
  }

  private catchUp(s: Sub, lastEventId: number | undefined): void {
    const oldest = this.ring[0]?.id ?? this.nextId;
    const replayable = lastEventId !== undefined && s.lifetime === this.lifetime && lastEventId >= oldest - 1 && lastEventId < this.nextId;
    if (replayable) {
      for (const e of this.ring) if (e.id > lastEventId!) this.deliver(s, e);
      return;
    }
    const reason = lastEventId !== undefined && s.lifetime === this.lifetime ? "expired" : "restart";
    s.lifetime = this.lifetime;
    s.lastId = this.lastId; // the client refetches snapshots as of now; later reconnects resume from here
    s.handlers.onFrame({ event: "reset", data: { reason } });
  }

  /** Subscribers that were refused by the gate try again (call after a successful pairing). */
  retryGated(): void {
    for (const s of this.subs) if (s.gated && !s.disposed) this.connect(s, undefined);
  }

  /**
   * The connection drops for `forMs`, then comes back the way a browser's EventSource does:
   * reconnecting with the last id it saw. Frames published meanwhile are replayed (or `reset`).
   */
  disconnect(forMs: number): void {
    const dropped = [...this.subs].filter((s) => s.online && !s.disposed);
    for (const s of dropped) {
      s.online = false;
      this.state(s, "connecting");
    }
    const t = this.opts.clock.setTimer(() => {
      this.timers.delete(t);
      for (const s of dropped) {
        if (s.disposed) continue;
        s.online = true;
        this.state(s, "open");
        this.catchUp(s, s.lastId);
      }
    }, forMs);
    this.timers.add(t);
  }

  /** The daemon restarted: new id sequence and an empty ring, so reconnects get `reset`. */
  newLifetime(): void {
    this.lifetime++;
    this.ring = [];
    this.nextId = 1;
  }

  /** For tests and the debug handle. */
  get size(): number {
    return this.ring.length;
  }
  get lastId(): number {
    return this.nextId - 1;
  }
}
