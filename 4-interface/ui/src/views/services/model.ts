// Pure logic for the service page and the shared time/word helpers (also used by Overview).
// No React, so the state matrix is unit-tested in the node environment.
import { isApiError } from "../../lib/api/problem";
import { classifyHref, kbRoutePath, normalizeKbPath } from "../../lib/markdown";
import { serviceGoal, type PromptGoal } from "../../lib/prompts";
import { own } from "../../lib/api/own";
import type { ServiceRecord } from "../../lib/api/types";

// ------------------------------------------------------------------------------------------ time

const ISO_TS = /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d+)?(?:Z|[+-]\d{2}:\d{2})$/;

/** "45 s", "12 min", "3 h 5 min", "2 d 4 h". */
export function formatDuration(totalSec: number): string {
  const s = Math.max(0, Math.floor(Number.isFinite(totalSec) ? totalSec : 0));
  if (s < 60) return `${s} s`;
  const m = Math.floor(s / 60);
  if (m < 60) return `${m} min`;
  const h = Math.floor(m / 60);
  if (h < 24) return m % 60 === 0 ? `${h} h` : `${h} h ${m % 60} min`;
  const d = Math.floor(h / 24);
  return h % 24 === 0 ? `${d} d` : `${d} d ${h % 24} h`;
}

/** "just now", "40 s ago", "3 min ago", "2 h ago", "5 d ago"; "unknown" for a missing or unparsable stamp. */
export function agoText(iso: string | null | undefined, now: number): string {
  const at = iso ? Date.parse(iso) : Number.NaN;
  if (Number.isNaN(at)) return "unknown";
  const sec = Math.floor((now - at) / 1000);
  if (sec < 10) return "just now";
  if (sec < 60) return `${sec} s ago`;
  const m = Math.floor(sec / 60);
  if (m < 60) return `${m} min ago`;
  const h = Math.floor(m / 60);
  if (h < 24) return `${h} h ago`;
  return `${Math.floor(h / 24)} d ago`;
}

/** "since 3 h ago"; "since just now" for a fresh change. */
export const sinceText = (iso: string | null | undefined, now: number): string => `since ${agoText(iso, now)}`;

// ----------------------------------------------------------------------------------- detail table

export interface DetailRow {
  label: string;
  value: string;
}

/** `model_id` -> "Model id", `lastProbe` -> "Last probe". */
export function humanizeKey(key: string): string {
  const spaced = key.replace(/[_-]+/g, " ").replace(/([a-z0-9])([A-Z])/g, (_m, a: string, b: string) => `${a} ${b.toLowerCase()}`).trim();
  return spaced === "" ? key : spaced.charAt(0).toUpperCase() + spaced.slice(1);
}

const MAX_ROWS = 40;

/** Labels for the `*_ago_s` keys the daemon sends; any other key of that shape is humanized without the suffix. */
const AGE_LABEL: Record<string, string> = { checked_ago_s: "Last check", last_flush_ago_s: "Last flush" };
const MAX_VALUE = 300;

const clip = (s: string): string => (s.length > MAX_VALUE ? `${s.slice(0, MAX_VALUE)}…` : s);

/** A scalar as display text, or null when it is empty (and the row is hidden). */
function scalarText(v: unknown, now: number): string | null {
  if (v === null || v === undefined) return null;
  if (typeof v === "boolean") return v ? "yes" : "no";
  if (typeof v === "number") return Number.isFinite(v) ? String(v) : null;
  if (typeof v === "string") {
    const t = v.trim();
    if (t === "") return null;
    return ISO_TS.test(t) ? agoText(t, now) : clip(t);
  }
  return null;
}

const isScalar = (v: unknown): boolean => v === null || ["string", "number", "boolean", "undefined"].includes(typeof v);
const isPlainObject = (v: unknown): v is Record<string, unknown> => typeof v === "object" && v !== null && !Array.isArray(v);

function compactJson(v: unknown): string | null {
  try {
    const s = JSON.stringify(v);
    return s === undefined ? null : clip(s);
  } catch {
    return null;
  }
}

/**
 * Registry `detail` as table rows: keys humanized, empty values hidden, nested objects flattened one level
 * ("probe": {"status": 204} -> "Probe status"), deeper structures shown as compact JSON.
 */
export function flattenDetail(detail: Record<string, unknown> | null | undefined, now: number = Date.now()): DetailRow[] {
  const rows: DetailRow[] = [];
  const seen = new Set<string>();
  const push = (label: string, value: string | null) => {
    if (value === null || value === "" || rows.length >= MAX_ROWS || seen.has(label)) return;
    seen.add(label);
    rows.push({ label, value });
  };
  const anyValue = (label: string, v: unknown) => {
    if (isScalar(v)) push(label, scalarText(v, now));
    else if (Array.isArray(v)) {
      if (v.length === 0) return;
      push(label, v.every(isScalar) ? clip(v.map((x) => scalarText(x, now)).filter((x): x is string => x !== null).join(", ")) : compactJson(v));
    } else push(label, compactJson(v));
  };
  for (const [key, v] of Object.entries(detail ?? {})) {
    // A monotonic age (`checked_ago_s`, `last_flush_ago_s`: the gateway row, Layer 7 section 3.5) reads as an age, under its own name.
    const aged = /^(.+)_ago_s$/.exec(key);
    if (aged && typeof v === "number") {
      push(AGE_LABEL[key] ?? humanizeKey(aged[1]!), ageAgoText(v));
      continue;
    }
    const label = humanizeKey(key);
    if (isPlainObject(v)) {
      for (const [k2, v2] of Object.entries(v)) anyValue(`${label} ${humanizeKey(k2).toLowerCase()}`, v2);
    } else anyValue(label, v);
  }
  return rows;
}

// --------------------------------------------------------------------------------- page state

export type ServiceBody = "live" | "starting" | "unset" | "unavailable" | "error";

export interface Handoff {
  /** `setup` / `fix`, or the Layer 5 context goal for the service that has one (`fabric`, `meshtastic`). */
  goal: PromptGoal;
  /** The button text: with the agent live it hands off; otherwise it reveals the request it would have sent. */
  label: string;
  mode: "agent" | "request";
}

export interface ServicePageState {
  body: ServiceBody;
  handoff: Handoff | null;
  /** Route path of the recipe (`/kb/...`), when the registry names one. */
  kbPath: string | null;
  /** Safe http(s) URL for the "open full client" link-out. */
  embedUrl: string | null;
  /** The advanced log panel is offered. */
  log: boolean;
  /** The registry lists `restart` (the daemon supervises this child): offer a Restart button. */
  restart: boolean;
  /**
   * The service is stuck in the supervisor's back-off (`child_degraded`): besides "Fix with the agent",
   * point at Settings -> Restart node, which resets the supervisor itself.
   */
  nodeRestartLink: boolean;
  /** The service is LIVE and carries a hosted panel (Layer 5, section 13): the page gains a Panel tab. */
  panelTab: boolean;
}

/**
 * Services with no agent hand-off: the gateway is the owner's own switch (Layer 7 section 3.5: Settings -> Layer 7 turns a carrier and a
 * gateway on; `gateway_none` and `gateway_off` are NOT SET until then), not something the agent sets up, so its row offers no "Set up with
 * the agent". Everything else that can run here and is unset offers it.
 */
export const NO_SETUP_SERVICES: ReadonlySet<string> = new Set(["gateway"]);

/**
 * With the agent down, the hand-off button shows the prepared message (the prompt and its context),
 * not instructions. The recipe in the knowledge base is where the actual steps are.
 */
export const REQUEST_LABEL = "Show the request";
export const REQUEST_HIDE_LABEL = "Hide the request";
export const REQUEST_COPY_LABEL = "copy the request";
export const requestNote = (hasRecipe: boolean): string =>
  hasRecipe
    ? "The agent is not running. This is the request it would get; the recipe has the steps."
    : "The agent is not running. This is the request it would get. Copy it for when the agent is back.";


export function servicePageState(service: ServiceRecord, agentLive: boolean): ServicePageState {
  const body: ServiceBody =
    service.state === "live" ? "live" : service.state === "starting" ? "starting" : service.state === "error" ? "error" : service.can_run_here ? "unset" : "unavailable";

  let handoff: Handoff | null = null;
  if (body === "unset" && !NO_SETUP_SERVICES.has(service.id)) {
    handoff = { goal: serviceGoal(service.id, "setup"), label: agentLive ? "Set up with the agent" : REQUEST_LABEL, mode: agentLive ? "agent" : "request" };
  } else if (body === "error") {
    handoff = { goal: serviceGoal(service.id, "fix"), label: agentLive ? "Fix with the agent" : REQUEST_LABEL, mode: agentLive ? "agent" : "request" };
  }

  const kb = service.kb_entry ? normalizeKbPath(service.kb_entry) : null;
  const embed = service.embed_url ? classifyHref(service.embed_url) : null;
  const running = body === "live" || body === "starting" || body === "error";
  return {
    body,
    handoff,
    kbPath: kb ? kbRoutePath(kb) : null,
    embedUrl: embed?.kind === "external" ? embed.href : null,
    log: running && service.actions.includes("log"),
    restart: (body === "live" || body === "error") && service.actions.includes("restart"),
    nodeRestartLink: body === "error" && service.reason_code === "child_degraded",
    panelTab: body === "live" && !!service.panel,
  };
}

// ------------------------------------------------------------------------------------ restart

/**
 * The Restart button's life. `sending`: clicked, the request is in flight. `waiting`: the node said
 * 202; the button keeps reading "Restarting…" until the next service list arrives (`rev` is the store's
 * `servicesRev` when the click happened).
 */
export type RestartPhase = { kind: "idle" } | { kind: "sending"; rev: number } | { kind: "waiting"; rev: number };

export const RESTART_IDLE: RestartPhase = { kind: "idle" };

/** Longest the button shows "Restarting…" without a service list arriving (a restart that changes nothing). */
export const RESTART_GIVE_UP_MS = 20_000;

export const beginRestart = (rev: number): RestartPhase => ({ kind: "sending", rev });

/** The node answered 202. */
export const acceptRestart = (p: RestartPhase): RestartPhase => (p.kind === "sending" ? { kind: "waiting", rev: p.rev } : p);

/** A service list arrived: a `waiting` restart is over once the list is newer than the click. */
export const settleRestart = (p: RestartPhase, rev: number): RestartPhase => (p.kind === "waiting" && rev > p.rev ? RESTART_IDLE : p);

export const restartButtonLabel = (p: RestartPhase): string => (p.kind === "idle" ? "Restart" : "Restarting…");

/** Plain text for a failed restart. A 409 `not_supervised` means the node does not run this service itself. */
export function restartErrorText(err: unknown): string {
  if (isApiError(err)) {
    if (err.code === "not_supervised") return "The node does not run this one itself, so it cannot restart it from here.";
    if (err.code === "network") return "The node could not be reached. Check the connection and try again.";
    if (err.code === "loopback_only") return "Available on the node itself.";
    return err.problem.detail || err.problem.title || "The node refused the request.";
  }
  return err instanceof Error && err.message ? err.message : "Something went wrong.";
}

// ------------------------------------------------------------------------------------- reasons

/**
 * Reason text for a NOT SET service that reads as "not done yet" instead of as a failure. The daemon's
 * wording for these ("...could not be set up") sounds like an error on a state that is just unfinished.
 * Keyed by registry `reason_code`; only used while the service is `unset`. Everything else is shown as sent.
 */
const NOT_SET_REASON: Record<string, string> = {
  venv_unavailable: "Python is installed. The Reticulum environment is not set up yet.",
  no_python: "Python is not installed on this machine yet. The mesh transport needs it.",
};

/**
 * What to show as the service's reason: the daemon's text, except for NOT SET codes reworded above and the `gateway` row, which is
 * always worded by `gatewayBadge` (Layer 7 section 3.8: the four codes, and `unknown` for anything else) so the page says exactly what the check supports. `id` and `detail` are
 * optional: a caller that has only the three reason fields gets the daemon's sentence for the gateway row too.
 */
export function reasonText(service: Pick<ServiceRecord, "state" | "reason" | "reason_code"> & Partial<Pick<ServiceRecord, "id" | "detail">>): string {
  // The gateway row is worded by the badge alone: a code outside the four (an older node's retired `wan_*`) reads `unknown`, never the node's own sentence.
  if (service.id === "gateway") return gatewayBadge(service.reason_code, service.detail);
  return (service.state === "unset" ? NOT_SET_REASON[service.reason_code] : undefined) ?? service.reason;
}

// ------------------------------------------------------------------------------ gateway badge

/**
 * The four states of the gateway badge (Layer 7 section 3.5 item 2, section 3.8), keyed by the `gateway` row's `reason_code`. The words
 * never say "online": the badge is the hub's own claim from its carrier's last answer, not a statement about the internet. A code this
 * table does not know (an older node still sends `wan_ok`) renders as `unknown`.
 */
const GATEWAY_BADGE = {
  gateway_none: "Not a gateway",
  carrier_ok: "Gateway: carrier reachable at last check",
  carrier_pending: "Gateway: last flush",
  gateway_off: "Gateway off (by you)",
} as const;

/** The metered line that sits beside the badge (section 12): past a metered link's daily cap no flush runs. */
export const METERED_CAP_TEXT = "Metered cap reached: no flush runs until the 24 hour allowance frees up.";

/** "just now", "45 s ago", "4 min ago": a monotonic age in seconds (the contract carries no wall-clock stamp); null for a missing or bad age. */
export function ageAgoText(sec: unknown): string | null {
  if (typeof sec !== "number" || !Number.isFinite(sec) || sec < 0) return null;
  return sec < 10 ? "just now" : `${formatDuration(sec)} ago`;
}

const count = (v: unknown): number => (typeof v === "number" && Number.isFinite(v) && v > 0 ? Math.floor(v) : 0);

/**
 * The badge as one sentence from the gateway row's `reason_code` and `detail` (`GatewayDetail`, section 3.5 item 3), the same for every
 * node. `carrier_pending` says what it is waiting on: items (`K waiting`) or, with nothing waiting, a check the carrier did not answer.
 */
export function gatewayBadge(code: string | null | undefined, detail?: Record<string, unknown> | null): string {
  if (typeof code !== "string" || !own(GATEWAY_BADGE, code)) return "unknown";
  const d = detail ?? {};
  switch (code) {
    case "carrier_ok": {
      const ago = ageAgoText(d.checked_ago_s);
      return ago ? `${GATEWAY_BADGE.carrier_ok}, ${ago}` : GATEWAY_BADGE.carrier_ok;
    }
    case "carrier_pending": {
      const waiting = count(d.pending);
      if (waiting === 0) {
        const ago = ageAgoText(d.checked_ago_s);
        return ago ? `Gateway: the carrier did not answer at the last check, ${ago}` : "Gateway: the carrier did not answer at the last check";
      }
      const flush = ageAgoText(d.last_flush_ago_s);
      return `${GATEWAY_BADGE.carrier_pending} ${flush ?? "not yet"}, ${waiting} waiting`;
    }
    default:
      return GATEWAY_BADGE[code];
  }
}

export const KIND_LABEL: Record<ServiceRecord["kind"], string> = {
  child: "runs as a program",
  detected: "detected",
  builtin: "built in",
};
