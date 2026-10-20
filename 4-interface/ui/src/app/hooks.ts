// Small browser hooks for the shell.
import { useEffect, useState, useSyncExternalStore } from "react";

/** Tailwind `lg`. Below it the sidebar is an off-canvas drawer. */
export const DESKTOP_QUERY = "(min-width: 1024px)";

export function useMediaQuery(query: string): boolean {
  const subscribe = (cb: () => void) => {
    if (typeof window === "undefined" || !window.matchMedia) return () => {};
    const mq = window.matchMedia(query);
    mq.addEventListener("change", cb);
    return () => mq.removeEventListener("change", cb);
  };
  const get = () => (typeof window !== "undefined" && !!window.matchMedia && window.matchMedia(query).matches);
  return useSyncExternalStore(subscribe, get, () => false);
}

/** Epoch ms, refreshed every `everyMs` while `active`. */
export function useNow(active: boolean, everyMs = 1000): number {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    if (!active) return;
    setNow(Date.now());
    const id = window.setInterval(() => setNow(Date.now()), everyMs);
    return () => window.clearInterval(id);
  }, [active, everyMs]);
  return now;
}
