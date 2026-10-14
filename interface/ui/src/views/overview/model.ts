// Pure derivations for Overview (spec 6.2): the Needs-you block, the Nearby sentence, the Devices
// summary, the Node facts and the activity lines. No React, no clock except what is passed in.
import type { AgentEvent, DeviceRecord, Proposal, ServiceRecord, Status } from "../../lib/api/types";
import type { DotState } from "../../components/status-dot";
import { IDENTITY_BACKUP_COMMAND } from "../../lib/commands";
import { displayName, summary } from "../../lib/devices";
import { sanitizeLine } from "../../lib/prompts";
import { agoText, formatDuration, reasonText } from "../services/model";

export const NEEDS_YOU_ID = "needs-you";

// ---------------------------------------------------------------------------------- needs you

export interface NeedItem {
  key: string;
  kind: "proposal" | "service" | "device";
  /** Title of the proposal, plain name of the service, alias or name of the device. */
  label: string;
  detail: string;
  /** Route path (no `#`). Proposals open the Agent view, where they render as cards. */
  href: string;
}

export interface ChecklistRow {
  id: "identity" | "model" | "transport" | "peer";
  title: string;
  state: DotState;
  text: string;
  /** Route path of the page that fixes it. */
  href?: string;
  /** A command to run on the node, shown with a copy button. */
  command?: string;
  done: boolean;
}

export interface NeedsYou {
  items: NeedItem[];
  checklist: ChecklistRow[];
  /** Something wants the user: an item, or a checklist row (other than the always-unknown identity backup) that is not done. */
  open: boolean;
}

function checklistFromService(id: ChecklistRow["id"], title: string, s: ServiceRecord | undefined, services: readonly ServiceRecord[] | null): ChecklistRow | null {
  if (!s) return { id, title, state: "unknown", text: services === null ? "waiting for the node" : "not reported by this node", done: false };
  if (s.can_run_here === false) return null; // this device cannot run it: not part of its checklist
  const href = `/services/${s.id}`;
  switch (s.state) {
    case "live":
      return { id, title, state: "live", text: "running", href, done: true };
    case "starting":
      return { id, title, state: "starting", text: "starting up", href, done: false };
    case "error":
      return { id, title, state: "error", text: s.reason, href, done: false };
    default:
      return { id, title, state: "unset", text: reasonText(s), href, done: false };
  }
}

export function needsYou(
  status: Status | null,
  services: readonly ServiceRecord[] | null,
  devices: readonly DeviceRecord[] | null,
  proposals: readonly Proposal[] | null,
): NeedsYou {
  const items: NeedItem[] = [];
  for (const p of proposals ?? []) {
    if (p.decision !== null) continue;
    items.push({ key: `proposal:${p.id}`, kind: "proposal", label: p.title, detail: `${p.kind} proposal from the agent`, href: "/" });
  }
  for (const s of services ?? []) {
    if (s.state === "error") items.push({ key: `service:${s.id}`, kind: "service", label: s.plain_name, detail: s.reason, href: `/services/${s.id}` });
  }
  for (const d of devices ?? []) {
    if (d.state === "error") items.push({ key: `device:${d.id}`, kind: "device", label: d.alias ?? d.name, detail: d.reason, href: `/devices/${encodeURIComponent(d.id)}` });
  }

  const find = (id: ServiceRecord["id"]) => services?.find((s) => s.id === id);
  const peerState = status?.transport_state;
  const checklist: ChecklistRow[] = [
    // Layer 2 has no "backed up" marker yet, so this stays honestly unknown.
    {
      id: "identity",
      title: "Identity backup",
      state: "unknown",
      text: "unknown — run the command below on the node",
      command: IDENTITY_BACKUP_COMMAND,
      done: false,
    },
  ];
  const model = checklistFromService("model", "Local model", find("model"), services);
  if (model) checklist.push(model);
  const transport = checklistFromService("transport", "Mesh transport", find("transport"), services);
  if (transport) checklist.push(transport);
  checklist.push(
    !status
      ? { id: "peer", title: "First nearby node heard", state: "unknown", text: "waiting for the node", done: false }
      : peerState === "joined"
        ? { id: "peer", title: "First nearby node heard", state: "live", text: "at least one nearby node has been heard", done: true }
        : {
            id: "peer",
            title: "First nearby node heard",
            state: "unset",
            text: peerState === "no transport" ? "needs the mesh transport first" : "none heard yet",
            done: false,
          },
  );

  return { items, checklist, open: items.length > 0 || checklist.some((r) => r.id !== "identity" && !r.done) };
}

// ----------------------------------------------------------------------------- nearby / devices

/** One sentence from `transport_state` (v1 knows nothing else about the neighbourhood). */
export function nearbySentence(status: Status | null): string {
  switch (status?.transport_state) {
    case "joined":
      return "At least one nearby node has been heard, so messages can flow.";
    case "isolated":
      return "No nearby nodes heard yet. This node is alone on the mesh right now.";
    case "no transport":
      return "This node has no mesh transport yet, so it cannot hear anyone. Ask the agent to set one up.";
    case "transport degraded":
      return "The mesh transport has a problem, so nearby nodes cannot be heard right now.";
    default:
      return "Waiting for the node to report.";
  }
}

export interface DevicesSummary {
  total: number;
  connected: number;
  needSetup: number;
  error: number;
  text: string;
  /**
   * Boards the node offers firmware for (Layer 6). They are already counted in `needSetup` and in `text`;
   * this only says which ones, so the card can tag them. Not a second counter.
   */
  offers: { id: string; name: string; state: DeviceRecord["state"] }[];
}

export function devicesSummary(devices: readonly DeviceRecord[] | null): DevicesSummary {
  const list = devices ?? [];
  const connected = list.filter((d) => d.state === "connected").length;
  const needSetup = list.filter((d) => d.state === "needs_setup").length;
  const error = list.filter((d) => d.state === "error").length;
  const text = devices === null ? "Waiting for the node to report." : list.length === 0 ? "Nothing attached or paired yet." : summary(list).sentence;
  const offers = list.filter((d) => !!d.flash_offer).map((d) => ({ id: d.id, name: displayName(d), state: d.state }));
  return { total: list.length, connected, needSetup, error, text, offers };
}

// ------------------------------------------------------------------------------------ node facts

export interface NodeFacts {
  name: string;
  label: string | null;
  bech32: string | null;
  alias: string | null;
  category: string | null;
  version: string;
  uptime: string;
  update: string;
  lastCheck: string;
  gateway: string;
  addresses: string[];
  /** Who can open the dashboard: "this device only" or "devices on your local network". */
  reach: string;
}

/** `runningVersion`: the version the node runs now, so an applied update can say what it updated to. */
export function updateText(u: Status["update"], runningVersion?: string | null): string {
  const v = u.pending_version ? ` ${u.pending_version}` : "";
  switch (u.state) {
    case "idle":
      if (u.pending) return `update${v} waiting`;
      if (u.last_result === "up_to_date") return "up to date";
      // `staged` is the last result of an update that went through; the node is idle again on the new version.
      if (u.last_result === "staged") return runningVersion ? `updated to ${runningVersion}` : "updated";
      return "idle";
    case "checking":
      return "checking for updates\u2026";
    case "staged":
      return `update${v} staged, applies on restart`;
    case "probation":
      return "new version on trial";
    case "reverting":
      return "going back to the previous version";
    case "failed":
      return "last update failed";
  }
}

export function nodeFacts(status: Status, services: readonly ServiceRecord[] | null, now: number): NodeFacts {
  const n = status.node;
  const gw = services?.find((s) => s.id === "gateway");
  return {
    name: n.alias ?? n.label ?? "This node",
    label: n.label,
    bech32: n.bech32,
    alias: n.alias,
    category: n.category,
    version: n.version,
    uptime: formatDuration(n.uptime_sec),
    update: updateText(status.update, n.version),
    lastCheck: status.update.last_check ? agoText(status.update.last_check, now) : "never",
    gateway: !gw ? "unknown" : gw.state === "live" ? "internet reachable" : gw.state === "starting" ? "checking" : "no internet",
    addresses: status.bind.urls,
    reach: status.bind.lan ? "devices on your local network can open it" : "this device only",
  };
}

// ----------------------------------------------------------------------------------- activity

const MAX_DETAIL = 120;

/** `key=value` pairs, control characters stripped, clipped to one line. */
export function detailText(detail: AgentEvent["detail"]): string {
  const parts: string[] = [];
  for (const [k, v] of Object.entries(detail ?? {})) {
    let value: string;
    if (typeof v === "string") value = v;
    else {
      try {
        value = JSON.stringify(v) ?? "";
      } catch {
        value = String(v);
      }
    }
    parts.push(`${sanitizeLine(k)}=${sanitizeLine(value)}`);
  }
  const line = parts.join(" ");
  return line.length > MAX_DETAIL ? `${line.slice(0, MAX_DETAIL)}…` : line;
}

const two = (n: number): string => String(n).padStart(2, "0");

/** `kind · 14:02:11 · task_id=... kind=diagnose`. Local time; the stamp is dropped when it cannot be read. */
export function activityLine(ev: AgentEvent): string {
  const d = new Date(ev.ts);
  const time = Number.isNaN(d.getTime()) ? "--:--:--" : `${two(d.getHours())}:${two(d.getMinutes())}:${two(d.getSeconds())}`;
  const detail = detailText(ev.detail);
  return detail ? `${ev.kind} · ${time} · ${detail}` : `${ev.kind} · ${time}`;
}

/** The newest `n` events, oldest first (a terminal reads top to bottom). `events` arrive newest first. */
export function recentActivity(events: readonly AgentEvent[], n: number): string[] {
  return events.slice(0, n).map(activityLine).reverse();
}
