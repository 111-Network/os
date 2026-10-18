// Navigation config (spec section 5). Pure data and pure functions: no React, so it is unit-tested
// in the node environment. The services section is dynamic (one row per registry record).
import type { DotState } from "../components/status-dot";
import type { UiMode } from "../components/spec-table";
import type { ServiceId, ServiceRecord, ServiceState, Status } from "./api/types";
import { LAYERS, layerBySlug } from "./layers";

export type NavSection = "core" | "services" | "advanced" | "settings";

export interface NavItem {
  id: string;
  label: string;
  /** Route path without the `#`, e.g. `/devices`. */
  href: string;
  /** Lowest mode that shows the row. Absent = always. */
  minMode?: UiMode;
  section: NavSection;
}

/** The label of the collapsible group in the sidebar. */
export const SETTINGS_LABEL = "Settings";

/** The one layer page shown in simple mode (Messaging & data, `/settings/l5`). */
export const SIMPLE_LAYER_PAGE = "l5";

export const NAV: readonly NavItem[] = [
  { id: "overview", label: "Overview", href: "/overview", section: "core" },
  { id: "agent", label: "Agent", href: "/", section: "core" },
  { id: "devices", label: "Devices", href: "/devices", section: "core" },
  { id: "nearby", label: "Nearby", href: "/nearby", section: "core" },
  { id: "messages", label: "Messages", href: "/messages", section: "core" },
  { id: "logs", label: "Logs", href: "/logs", minMode: "advanced", section: "advanced" },
  { id: "settings", label: "Essentials", href: "/settings", section: "settings" },
  ...LAYERS.map(
    (l): NavItem => ({
      id: `settings-${l.slug}`,
      label: `${l.title} · L${l.n}`,
      href: `/settings/${l.slug}`,
      // Layer 5's page holds the first step of messaging (enroll with a hub), and Messages is a core view: it is
      // reachable in both modes. Every other layer page is advanced.
      ...(l.slug === SIMPLE_LAYER_PAGE ? {} : { minMode: "advanced" as const }),
      section: "settings",
    }),
  ),
];

export interface ServiceNavItem {
  id: ServiceId;
  /** The plain name, e.g. "Local AI agent". */
  label: string;
  href: string;
  state: ServiceState;
}

export interface VisibleNav {
  core: NavItem[];
  /** Services that can run on this device, in registry order. */
  services: ServiceNavItem[];
  /** `can_run_here === false`: the collapsed "Not available on this device (n)" group. */
  unavailable: ServiceNavItem[];
  advanced: NavItem[];
  settings: NavItem[];
}

/** Does `mode` satisfy a row's `minMode`? */
export function meetsMode(mode: UiMode, minMode: UiMode | undefined): boolean {
  return minMode === undefined || minMode === "simple" || mode === "advanced";
}

export function serviceHref(id: string): string {
  return `/services/${id}`;
}

function serviceItem(s: ServiceRecord): ServiceNavItem {
  return { id: s.id, label: s.plain_name, href: serviceHref(s.id), state: s.state };
}

/** Everything the sidebar renders for `mode`. `services` is null until the first snapshot arrives. */
export function visibleNav(mode: UiMode, services: readonly ServiceRecord[] | null): VisibleNav {
  const rows = NAV.filter((i) => meetsMode(mode, i.minMode));
  const list = services ?? [];
  return {
    core: rows.filter((i) => i.section === "core"),
    services: list.filter((s) => s.can_run_here !== false).map(serviceItem),
    unavailable: list.filter((s) => s.can_run_here === false).map(serviceItem),
    advanced: rows.filter((i) => i.section === "advanced"),
    settings: rows.filter((i) => i.section === "settings"),
  };
}

/** Disconnected means every dot goes hollow ("unknown"): the last known state is no longer proof. */
export function dotState(state: ServiceState, disconnected: boolean): DotState {
  return disconnected ? "unknown" : state;
}

/** The node chip dot: overall health. Unknown while there is no status, or the connection is down. */
export function healthDot(status: Pick<Status, "health"> | null, disconnected: boolean): DotState {
  if (!status || disconnected) return "unknown";
  switch (status.health.status) {
    case "ok":
      return "live";
    case "error":
      return "error";
    default:
      return "unknown";
  }
}

/** Is the sidebar row for `href` the current page? `/` and `/settings` match exactly; others also match children. */
export function isActive(href: string, path: string): boolean {
  if (href === path) return true;
  if (href === "/" || href === "/settings") return false;
  return path.startsWith(`${href}/`);
}

/** Two-letter glyph for the icon rail: the first two letters of the label, uppercased. */
export function glyphOf(label: string): string {
  return label.replace(/[^A-Za-z0-9]/g, "").slice(0, 2).toUpperCase();
}

/** "Node2A" becomes "Node 2A" for the node chip badge. */
export function categoryLabel(category: string): string {
  return category.replace(/^([A-Za-z]+)(\d)/, "$1 $2");
}

/** The lowest mode a route is meant for, or undefined when it is for everyone. Used for the "advanced view" badge. */
export function minModeFor(path: string): UiMode | undefined {
  const item = NAV.find((i) => i.href === path);
  if (item) return item.minMode;
  const layer = /^\/settings\/(l\d)$/.exec(path)?.[1];
  return layer && layerBySlug(layer) ? "advanced" : undefined;
}

/** Header title for a route. `services` resolves `/services/:id` to the plain name. */
export function titleFor(path: string, services?: readonly ServiceRecord[] | null): string {
  if (path === "/") return "Agent";
  const exact: Record<string, string> = {
    "/overview": "Overview",
    "/devices": "Devices",
    "/devices/add": "Add device",
    "/nearby": "Nearby",
    "/messages": "Messages",
    "/messages/contacts": "Contacts",
    "/logs": "Logs",
    "/settings": "Settings",
    "/settings/licenses": "Licenses",
    "/pair": "Pair",
    "/kb": "Knowledge base",
    "/_gallery": "Gallery",
  };
  const hit = exact[path];
  if (hit) return hit;
  if (path.startsWith("/kb/")) return "Knowledge base";
  const [, head, rest] = /^\/([^/]+)\/([^/]+)$/.exec(path) ?? [];
  switch (head) {
    case "devices":
      return "Device";
    case "messages":
      return "Conversation";
    case "services": {
      const rec = services?.find((s) => s.id === rest);
      return rec?.plain_name ?? "Service";
    }
    case "settings": {
      const layer = layerBySlug(rest);
      return layer ? `${layer.title} · L${layer.n}` : "Settings";
    }
    default:
      return "Not found";
  }
}
