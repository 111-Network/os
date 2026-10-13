// One shared visually-hidden aria-live="polite" region for throttled status announcements
// (spec section 3). Anything that wants to tell a screen reader something calls `announce(text)`.
import { useEffect, useRef, useState } from "react";
import { useApiMeta, useConnection } from "../lib/api/hooks";
import { useAppLocation } from "./location";

export const ANNOUNCE_INTERVAL_MS = 2000;
export const DISCONNECTED_TEXT = "Disconnected from the node. Showing the last known state.";
export const RECONNECTED_TEXT = "Reconnected to the node.";

export interface ConnectionView {
  disconnected: boolean;
  /** The stream has been open at least once this page load. */
  everOpened: boolean;
  /** The node wants a pair code: the shell shows that instead of a "last known state". */
  needsPair: boolean;
}

/**
 * What to tell a screen reader when the connection changes. `announced` is whether "Disconnected" is
 * the last thing said. Only a connection that existed can be lost, so a visitor who never got in
 * (unpaired) hears nothing; and "Reconnected" is said whenever the stream is back after a "Disconnected".
 */
export function connectionAnnouncement(announced: boolean, v: ConnectionView): { text: string | null; announced: boolean } {
  if (announced && !v.disconnected) return { text: RECONNECTED_TEXT, announced: false };
  if (!announced && v.disconnected && v.everOpened && !v.needsPair) return { text: DISCONNECTED_TEXT, announced: true };
  return { text: null, announced };
}

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

  // Connection changes. This component outlives every route (the header does not: /pair has none),
  // so "Disconnected" can still be followed by "Reconnected" after pairing.
  const { disconnected, everOpened } = useConnection();
  const { needsPair } = useApiMeta();
  const announced = useRef(false);
  useEffect(() => {
    const next = connectionAnnouncement(announced.current, { disconnected, everOpened, needsPair });
    announced.current = next.announced;
    if (next.text) announce(next.text);
  }, [disconnected, everOpened, needsPair]);

  // A new route starts with a clean region: a stale message must not follow the user around. Anything
  // announced during the same render pass (a view announcing on mount) is newer and stays.
  const [path] = useAppLocation();
  const shown = useRef(text);
  shown.current = text;
  const lastPath = useRef(path);
  useEffect(() => {
    if (lastPath.current === path) return;
    lastPath.current = path;
    const stale = shown.current;
    setText((current) => (current === stale ? "" : current));
  }, [path]);
  return (
    <div role="status" aria-live="polite" aria-atomic="true" className="sr-only">
      {text}
    </div>
  );
}
