// Pure logic for the service page and the shared time/word helpers (also used by Overview).
// No React, so the state matrix is unit-tested in the node environment.
import { classifyHref, kbRoutePath, normalizeKbPath } from "../../lib/markdown";
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
  goal: "setup" | "fix";
  /** The button text: with the agent live it hands off; otherwise it reveals the steps. */
  label: string;
  mode: "agent" | "steps";
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
  /** A restart action exists. There is no per-service restart route in v1, so it renders disabled. */
  restart: boolean;
}

/**
 * Services with nothing to set up: the gateway turns live by itself once the node reaches the internet
 * (spec 7: "no setup action"). Everything else that can run here and is unset offers "Set up with the agent".
 */
export const NO_SETUP_SERVICES: ReadonlySet<string> = new Set(["gateway"]);

export const RESTART_UNAVAILABLE = "per-service restart arrives with Layer 9";

export function servicePageState(service: ServiceRecord, agentLive: boolean): ServicePageState {
  const body: ServiceBody =
    service.state === "live" ? "live" : service.state === "starting" ? "starting" : service.state === "error" ? "error" : service.can_run_here ? "unset" : "unavailable";

  let handoff: Handoff | null = null;
  if (body === "unset" && !NO_SETUP_SERVICES.has(service.id)) {
    handoff = { goal: "setup", label: agentLive ? "Set up with the agent" : "Show steps", mode: agentLive ? "agent" : "steps" };
  } else if (body === "error") {
    handoff = { goal: "fix", label: agentLive ? "Fix with the agent" : "Show steps", mode: agentLive ? "agent" : "steps" };
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
  };
}

export const KIND_LABEL: Record<ServiceRecord["kind"], string> = {
  child: "runs as a program",
  detected: "detected",
  builtin: "built in",
};
