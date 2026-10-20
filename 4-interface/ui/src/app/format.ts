// Small pure text helpers for the shell banners.

/** Seconds since an event as short text: "12s ago", "3m ago", "2h ago". */
export function formatAgo(seconds: number): string {
  const s = Math.max(0, Math.floor(seconds));
  if (s < 60) return `${s}s ago`;
  const m = Math.floor(s / 60);
  if (m < 60) return `${m}m ago`;
  return `${Math.floor(m / 60)}h ago`;
}

/** Banner text for a lost connection. `lastEventAt` is epoch ms, or null before any sign of life. */
export function disconnectedText(lastEventAt: number | null, now: number): string {
  if (lastEventAt === null) return "Disconnected — no update received yet";
  return `Disconnected — last update ${formatAgo((now - lastEventAt) / 1000)}`;
}
