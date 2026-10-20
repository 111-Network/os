// Panels slice (os/5-messaging/README.md §13, §15): helpers over the declared hosted panels. A panel lives on its own
// origin `http://<host as typed>:<port>`; the dashboard builds that from `location.hostname` and the panel's `port`,
// never from a shared snapshot, and frames only registry-declared panels.
import type { HostedPanel, ServiceId } from "./types";

export const panelFor = (panels: readonly HostedPanel[] | null | undefined, id: ServiceId): HostedPanel | null => panels?.find((p) => p.id === id) ?? null;

/** Where the dashboard page is served from, for building a panel origin and judging its secure context. */
export interface PanelLocation {
  protocol: string;
  hostname: string;
}

/** `http://127.0.0.1:11190`: the dashboard's own hostname with the panel's port. IPv6 literals keep their brackets. */
export function panelOrigin(panel: Pick<HostedPanel, "port">, loc: PanelLocation): string {
  const host = loc.hostname.includes(":") && !loc.hostname.startsWith("[") ? `[${loc.hostname}]` : loc.hostname;
  return `${loc.protocol === "https:" ? "https:" : "http:"}//${host}:${panel.port}`;
}

/** Loopback hosts are potentially trustworthy origins (secure contexts) even over plain HTTP. */
export function isLoopbackHost(hostname: string): boolean {
  const h = hostname.replace(/^\[|\]$/g, "").toLowerCase();
  return h === "localhost" || h.endsWith(".localhost") || h === "::1" || /^127(\.\d{1,3}){3}$/.test(h);
}

/**
 * Is the dashboard origin potentially trustworthy (HTTPS or loopback)? A panel with `needs_secure_context` is not framed
 * otherwise: the tab says so and points at the node itself (§13, never faked).
 */
export const isSecureOrigin = (loc: PanelLocation): boolean => loc.protocol === "https:" || isLoopbackHost(loc.hostname);

export function panelShowsFrame(panel: Pick<HostedPanel, "needs_secure_context" | "state">, loc: PanelLocation): boolean {
  return panel.state.enabled && panel.state.listening && (!panel.needs_secure_context || isSecureOrigin(loc));
}
