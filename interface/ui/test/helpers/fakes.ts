// Fakes for the injected dependencies: clock/timers, EventSource, byte streams, Response builders.
import type { EventSourceCtor, EventSourceLike } from "../../src/lib/api/events-client";

export class FakeClock {
  t: number;
  private timers = new Map<number, { at: number; fn: () => void }>();
  private seq = 1;
  constructor(start = 1_000_000) {
    this.t = start;
  }
  now = (): number => this.t;
  setTimer = (fn: () => void, ms: number): unknown => {
    const id = this.seq++;
    this.timers.set(id, { at: this.t + ms, fn });
    return id;
  };
  clearTimer = (h: unknown): void => {
    this.timers.delete(h as number);
  };
  /** Move time forward, firing due timers in order (timers may schedule more timers). */
  advance(ms: number): void {
    const end = this.t + ms;
    for (;;) {
      let next: [number, { at: number; fn: () => void }] | null = null;
      for (const e of this.timers) if (e[1].at <= end && (!next || e[1].at < next[1].at)) next = e;
      if (!next) break;
      this.timers.delete(next[0]);
      this.t = Math.max(this.t, next[1].at);
      next[1].fn();
    }
    this.t = end;
  }
  /** Advance in steps, letting microtasks (queueMicrotask, promise continuations) run in between. */
  async advanceAsync(ms: number, step = 100): Promise<void> {
    for (let left = ms; left > 0; left -= step) {
      this.advance(Math.min(step, left));
      await flush();
    }
  }
  get pending(): number {
    return this.timers.size;
  }
}

/** Let promise continuations and queued microtasks run. */
export async function flush(times = 8): Promise<void> {
  for (let i = 0; i < times; i++) await Promise.resolve();
}

type Listener = (ev: { data?: unknown; lastEventId?: string }) => void;

export class FakeEventSource implements EventSourceLike {
  static CONNECTING = 0;
  static OPEN = 1;
  static CLOSED = 2;
  readyState = 0;
  closed = false;
  private listeners = new Map<string, Listener[]>();
  constructor(
    public url: string,
    registry: FakeEventSource[],
  ) {
    registry.push(this);
  }
  addEventListener(type: string, listener: Listener): void {
    const l = this.listeners.get(type) ?? [];
    l.push(listener);
    this.listeners.set(type, l);
  }
  close(): void {
    this.closed = true;
    this.readyState = 2;
  }
  private fire(type: string, ev: { data?: unknown; lastEventId?: string } = {}): void {
    for (const l of this.listeners.get(type) ?? []) l(ev);
  }
  open(): void {
    this.readyState = 1;
    this.fire("open");
  }
  /** Connection error. `fatal` = the browser gave up (readyState CLOSED); else it will retry. */
  error(fatal = false): void {
    this.readyState = fatal ? 2 : 0;
    this.fire("error");
  }
  message(name: string, data: unknown, lastEventId?: string): void {
    this.fire(name, { data: typeof data === "string" ? data : JSON.stringify(data), ...(lastEventId ? { lastEventId } : {}) });
  }
  hasListener(type: string): boolean {
    return (this.listeners.get(type)?.length ?? 0) > 0;
  }
}

export function fakeEventSourceCtor(): { ctor: EventSourceCtor; instances: FakeEventSource[] } {
  const instances: FakeEventSource[] = [];
  const ctor = class extends FakeEventSource {
    constructor(url: string) {
      super(url, instances);
    }
  };
  return { ctor: ctor as unknown as EventSourceCtor, instances };
}

const enc = new TextEncoder();
export const bytes = (s: string): Uint8Array => enc.encode(s);

/** An async iterable of the given chunks. */
export async function* chunksOf(chunks: Uint8Array[]): AsyncGenerator<Uint8Array> {
  for (const c of chunks) yield c;
}

/** Split text's UTF-8 bytes at the given byte offsets. */
export function splitBytes(text: string, offsets: number[]): Uint8Array[] {
  const all = bytes(text);
  const cuts = [0, ...offsets.filter((o) => o > 0 && o < all.length), all.length];
  const out: Uint8Array[] = [];
  for (let i = 0; i < cuts.length - 1; i++) out.push(all.slice(cuts[i]!, cuts[i + 1]!));
  return out;
}

/**
 * A ReadableStream that emits the chunks, with an optional cancel spy. With `hang` it stays open
 * after the last chunk, like a live connection, so cancellation by the consumer is observable.
 */
export function streamOf(chunks: Uint8Array[], onCancel?: () => void, hang = false): ReadableStream<Uint8Array> {
  let i = 0;
  return new ReadableStream<Uint8Array>({
    pull(controller) {
      if (i < chunks.length) controller.enqueue(chunks[i++]!);
      else if (!hang) controller.close();
      else return new Promise<void>(() => undefined);
    },
    cancel() {
      onCancel?.();
    },
  });
}

export function sseResponse(chunks: Uint8Array[], onCancel?: () => void, init: ResponseInit = {}, hang = false): Response {
  return new Response(streamOf(chunks, onCancel, hang), { status: 200, ...init, headers: { "content-type": "text/event-stream", ...(init.headers as Record<string, string> | undefined) } });
}

export const jsonResponse = (body: unknown, status = 200, headers: Record<string, string> = {}): Response =>
  new Response(JSON.stringify(body), { status, headers: { "content-type": "application/json", ...headers } });

export const problemResponse = (status: number, code: string, title = "Problem", detail?: string): Response =>
  new Response(JSON.stringify({ type: "about:blank", title, status, code, ...(detail ? { detail } : {}) }), {
    status,
    headers: { "content-type": "application/problem+json" },
  });

export async function collect<T>(it: AsyncIterable<T>): Promise<T[]> {
  const out: T[] = [];
  for await (const v of it) out.push(v);
  return out;
}

/** Array.prototype.at(-1) is outside the ES2020 lib target. */
export const last = <T>(a: readonly T[]): T | undefined => a[a.length - 1];
