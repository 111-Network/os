// One shared visually-hidden aria-live="polite" region for throttled status announcements
// (spec section 3). Anything that wants to tell a screen reader something calls `announce(text)`.
import { useEffect, useState } from "react";

export const ANNOUNCE_INTERVAL_MS = 2000;

export interface AnnouncerDeps {
  /** Receives each message that is let through. */
  emit: (text: string) => void;
  now?: () => number;
  setTimer?: (fn: () => void, ms: number) => unknown;
  intervalMs?: number;
}

/**
 * At most one message per interval. A message that arrives early is held and sent when the interval
 * ends; a newer held message replaces an older one (screen readers only need the latest state).
 */
export function createAnnouncer(deps: AnnouncerDeps): (text: string) => void {
  const now = deps.now ?? (() => Date.now());
  const setTimer = deps.setTimer ?? ((fn, ms) => setTimeout(fn, ms));
  const interval = deps.intervalMs ?? ANNOUNCE_INTERVAL_MS;
  let lastAt = Number.NEGATIVE_INFINITY;
  let held: string | null = null;
  let timer: unknown = null;

  const send = (text: string) => {
    lastAt = now();
    deps.emit(text);
  };

  return (text: string) => {
    const wait = lastAt + interval - now();
    if (wait <= 0 && timer === null) {
      send(text);
      return;
    }
    held = text;
    if (timer !== null) return;
    timer = setTimer(() => {
      timer = null;
      const next = held;
      held = null;
      if (next !== null) send(next);
    }, Math.max(0, wait));
  };
}

const listeners = new Set<(text: string) => void>();
const shared = createAnnouncer({ emit: (text) => listeners.forEach((l) => l(text)) });

/** Queue a polite announcement (throttled to one per 2 s). Safe to call before the region mounts. */
export function announce(text: string): void {
  shared(text);
}

export function LiveRegion() {
  const [text, setText] = useState("");
  useEffect(() => {
    listeners.add(setText);
    return () => {
      listeners.delete(setText);
    };
  }, []);
  return (
    <div role="status" aria-live="polite" aria-atomic="true" className="sr-only">
      {text}
    </div>
  );
}
