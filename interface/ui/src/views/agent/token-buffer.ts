// Batches streamed tokens so React state changes at most 10 times a second (spec 6.1).
//
// Pure: no timers and no React. The caller owns scheduling: after `push()`, ask `waitMs()` how long
// until a flush is allowed and call `flush()` then. The first flush after a quiet period is
// immediate (leading edge); later ones are at least `intervalMs` apart.

export const FLUSH_INTERVAL_MS = 100; // 10 Hz

export class TokenBuffer {
  private all = "";
  private flushed = 0; // length of `all` already handed out
  private lastFlushAt = Number.NEGATIVE_INFINITY;

  constructor(
    private readonly now: () => number,
    readonly intervalMs: number = FLUSH_INTERVAL_MS,
  ) {}

  push(chunk: string): void {
    this.all += chunk;
  }

  /** Everything received so far, flushed or not. */
  get text(): string {
    return this.all;
  }

  /** True when text has arrived that no flush has handed out yet. */
  get dirty(): boolean {
    return this.all.length > this.flushed;
  }

  /** Milliseconds until `flush()` may hand out text. 0 means now. Infinity when nothing is waiting. */
  waitMs(): number {
    if (!this.dirty) return Number.POSITIVE_INFINITY;
    return Math.max(0, this.lastFlushAt + this.intervalMs - this.now());
  }

  /** The full text so far when a flush is due, else null. */
  flush(): string | null {
    if (this.waitMs() > 0) return null;
    return this.take();
  }

  /** The full text so far, regardless of the rate limit (used when the stream ends). */
  take(): string {
    this.flushed = this.all.length;
    this.lastFlushAt = this.now();
    return this.all;
  }

  reset(): void {
    this.all = "";
    this.flushed = 0;
    this.lastFlushAt = Number.NEGATIVE_INFINITY;
  }
}
