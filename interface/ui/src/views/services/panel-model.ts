// The Panel tab's pure logic (os/messaging/README.md section 13, interface README 6.6): when a hosted panel is framed
// and when it is not (and what the page says instead), the iframe's sandbox and permission attributes, the ticket URL the
// frame may open, and the chrome's copy. No React and no clock, so the state matrix is unit-tested in the node environment.
//
// The rule the page keeps: the dashboard frames only a registry-declared panel, on its own origin, built from
// `location.hostname` and `panel.port`; the one-time ticket URL goes straight into the frame and is never shown.
import { own } from "../../lib/api/own";
import { isSecureOrigin, panelOrigin, type PanelLocation } from "../../lib/api/panels";
import { isApiError } from "../../lib/api/problem";
import type { HostedPanel, ServiceId } from "../../lib/api/types";

// --------------------------------------------------------------------------------------------- the frame

/**
 * The iframe's sandbox (section 13): scripts, same-origin (the panel's own origin, not ours), forms, popups, modals,
 * downloads. Never `allow-top-navigation`: a panel cannot move the dashboard.
 */
export const PANEL_SANDBOX = "allow-scripts allow-same-origin allow-forms allow-popups allow-modals allow-downloads";

const FEATURE = /^[a-z][a-z0-9-]*$/;

/**
 * The iframe's `allow` attribute from `panel.allow`. A bare feature name is scoped to the frame's own origin by the
 * browser (`allow="serial"` means `serial 'src'`), which is the "origin-scoped" of section 13. Anything that is not a
 * plain feature name (a value that tries to widen the policy to `*` or another origin) is dropped. null: no attribute.
 */
export function allowAttr(allow: readonly string[]): string | null {
  const seen = new Set<string>();
  for (const f of allow) {
    const name = typeof f === "string" ? f.trim() : "";
    if (FEATURE.test(name)) seen.add(name);
  }
  return seen.size > 0 ? [...seen].join("; ") : null;
}

/** Where the dashboard page is served from: `PanelLocation` plus the port (empty for the scheme's default). */
export interface PanelEnv extends PanelLocation {
  port: string;
}

/** The environment of the page now; a fixed one for tests and server rendering. */
export function currentEnv(): PanelEnv {
  if (typeof window === "undefined") return { protocol: "http:", hostname: "127.0.0.1", port: "11180" };
  return { protocol: window.location.protocol, hostname: window.location.hostname, port: window.location.port };
}

/** The dashboard on the node itself, for "open it there" (the node's loopback address; no TLS in v1). */
export function loopbackDashboardUrl(env: PanelEnv, id: ServiceId): string {
  const port = env.port !== "" && env.port !== "80" ? `:${env.port}` : "";
  return `http://127.0.0.1${port}/#/services/${id}`;
}

const bare = (host: string): string => host.replace(/^\[|\]$/g, "").toLowerCase();

/**
 * The ticket URL the frame may load, or null. `POST /panels/{id}/session` returns a URL on the host the browser typed;
 * the frame only ever loads the panel origin this page computes itself (`location.hostname` plus `panel.port`), so a URL
 * for any other host or port, any other scheme, or with credentials in it is refused rather than rewritten. A relative
 * URL resolves against that origin. The fragment is dropped.
 */
export function ticketSrc(raw: string, panel: Pick<HostedPanel, "port">, loc: PanelLocation): string | null {
  const origin = panelOrigin(panel, loc);
  let u: URL;
  try {
    u = new URL(raw, `${origin}/`);
  } catch {
    return null;
  }
  if (u.protocol !== "http:" && u.protocol !== "https:") return null;
  if (u.username !== "" || u.password !== "") return null;
  if (bare(u.hostname) !== bare(loc.hostname)) return null;
  if (u.port !== String(panel.port)) return null;
  return `${origin}${u.pathname}${u.search}`;
}

// ------------------------------------------------------------------------------------ what the tab shows

export type PanelGate =
  | { kind: "frame" }
  /** The panel has no listener: it is not enabled on this node. */
  | { kind: "off"; text: string }
  /** Enabled, but the listener is not up (starting, or it could not bind). */
  | { kind: "not_listening"; text: string }
  /** `needs_secure_context` and the dashboard origin is not potentially trustworthy: open it on the node itself. */
  | { kind: "secure_context"; text: string; url: string };

export const SECURE_CONTEXT_TEXT =
  "This panel needs a secure page, and this dashboard is not one: a browser treats this address as insecure, so the panel would not work properly here. Open the dashboard on the node itself, or over HTTPS, and come back to this tab.";

export const PANEL_OFF_TEXT = "This panel is not switched on for this node, so nothing is listening for it yet.";
export const PANEL_NOT_LISTENING_TEXT = "This panel is switched on, but its listener is not up yet. It opens by itself when it is; try again in a moment.";

/** Does the tab frame the panel, and if not, why. Order: off, not listening, then the secure-context gate. */
export function panelGate(panel: Pick<HostedPanel, "needs_secure_context" | "state">, env: PanelEnv, id: ServiceId): PanelGate {
  if (!panel.state.enabled) return { kind: "off", text: PANEL_OFF_TEXT };
  if (!panel.state.listening) return { kind: "not_listening", text: PANEL_NOT_LISTENING_TEXT };
  if (panel.needs_secure_context && !isSecureOrigin(env)) return { kind: "secure_context", text: SECURE_CONTEXT_TEXT, url: loopbackDashboardUrl(env, id) };
  return { kind: "frame" };
}

// ----------------------------------------------------------------------------------- the ticket, minted

export type MintFailure = "loopback_only" | "not_found" | "network" | "bad_url" | "other";

/** Why the ticket could not be minted, from the node's own answer. A 403 `loopback_only` is a loopback-only panel opened over the LAN. */
export function classifyMint(err: unknown): MintFailure {
  if (!isApiError(err)) return "other";
  if (err.code === "loopback_only") return "loopback_only";
  if (err.code === "not_found") return "not_found";
  if (err.code === "network") return "network";
  return "other";
}

export const LOOPBACK_ONLY_PANEL_TEXT =
  "This panel only opens on the node itself. You are connected over the network, and its listener stays on the node's own address, so it is not offered here.";

/** The sentence under a failed ticket mint; `detail` is the node's own words when it gave any. */
export function mintFailureText(why: MintFailure, detail?: string): string {
  switch (why) {
    case "loopback_only":
      return LOOPBACK_ONLY_PANEL_TEXT;
    case "not_found":
      return "The node has no panel for this service right now. It appears once the service is set up.";
    case "network":
      return "The node could not be reached, so no session could be opened. Check the connection and try again.";
    case "bad_url":
      return "The node gave an address for the panel that this page would not open, so nothing was framed. Reload the dashboard and try again.";
    default:
      return detail && detail.trim() !== "" ? detail : "The node refused to open a session for this panel.";
  }
}

/** A panel ticket lives 30 s (section 13); this is how long the page waits for the frame to load before it says so. */
export const LOAD_GRACE_S = 5;

// --------------------------------------------------------------------------------------- chrome copy

/** The frame's title: the manifest's title, else the service's plain name. */
export const panelTitle = (panel: Pick<HostedPanel, "title">, plainName: string): string => (panel.title && panel.title.trim() !== "" ? panel.title.trim() : plainName);

/** The health dot of the panel's upstream (its child state; never a connect). */
export function healthDot(health: HostedPanel["state"]["health"]): "live" | "error" | "unknown" {
  return health === "up" ? "live" : health === "down" ? "error" : "unknown";
}

/** Shown above the frame when the service behind the panel is down: the panel still opens, but it cannot reach its radio. */
export const UPSTREAM_DOWN_TEXT = "The service behind this panel is not answering right now. The panel opens, but it cannot reach your radio until the service is back.";

/** The off-origin hosts a browser with internet access will contact while the panel is open (verified at the offline lint). */
export function optionalHostsText(hosts: readonly string[]): string | null {
  const list = hosts.filter((h) => typeof h === "string" && h.trim() !== "");
  if (list.length === 0) return null;
  return `With an internet connection, this browser may contact ${list.join(", ")} while the panel is open. Without internet nothing breaks: ${list.length === 1 ? "that part stays" : "those parts stay"} blank (for example the map).`;
}

/**
 * Honest copy per panel, keyed by the panel id (= the service id). The first sentence of `transmits` is the one section 14
 * requires on the service page; `keys` is the "nothing here is 111-sealed" line; `residual` is the Termux note (D16 a).
 */
export const PANEL_NOTES: Readonly<Record<string, { transmits: string; keys: string; residual: string }>> = {
  meshtastic: {
    transmits:
      "What you do in this panel (send a message, change a setting, reboot the board) goes to your radio and out over Meshtastic. 111 does not see it and does not store it.",
    keys: "Meshtastic protects messages with its own keys; the default channel's key is public. Nothing here is 111-sealed.",
    residual:
      "If this node runs on a phone, every app on that phone can reach the connection meshmud keeps open on the phone's own address. For a Wi-Fi board that is the same exposure the board already has on your network.",
  },
};

export function panelNotes(id: string): { transmits: string; keys: string; residual: string } | null {
  return own(PANEL_NOTES, id) ? PANEL_NOTES[id]! : null;
}
