// Pure model for Settings (spec section 6.7): the Essentials row list, the loopback-only rule, the
// per-layer status rows and the "ask the agent" prompt. No React, so it runs in the node test
// environment. The views map these plain values onto primitives.
import { isApiError } from "../../lib/api/problem";
import type { ServiceRecord, Settings, Status } from "../../lib/api/types";
import { questionPrompt, type PromptGoal } from "../../lib/prompts";

/** Text shown in place of a control that only works on the node itself (paired LAN session). */
export const LOOPBACK_ONLY_TEXT = "available on the node itself";

// ---------------------------------------------------------------- essentials

export type EssentialId = "alias" | "mode" | "identity-backup" | "restart" | "update-check";

export interface EssentialRow {
  id: EssentialId;
  label: string;
  /** One or two plain sentences under the label. */
  help: string;
  /** Extra sentence shown below the control (identity backup only). */
  note?: string;
}

export const BACKUP_NOTE = "The dashboard never shows or sends your keys.";

const ESSENTIALS: readonly EssentialRow[] = [
  {
    id: "alias",
    label: "Node name",
    help: "A friendly name for this node, shown in the sidebar and the agent prompts. Leave it empty to use the node label. It does not change the node's identity.",
  },
  {
    id: "mode",
    label: "View mode",
    help: "Simple shows what you need day to day. Advanced adds logs, the layer pages and raw detail. This only changes what this dashboard shows.",
  },
  {
    id: "identity-backup",
    label: "Identity backup",
    help: "Your identity is 12 words. Back them up once, from the node itself:",
    note: BACKUP_NOTE,
  },
  {
    id: "restart",
    label: "Restart node",
    help: "Restarts the node's daemon. This page disconnects for a moment and reconnects on its own.",
  },
  {
    id: "update-check",
    label: "Check for updates",
    help: "Asks the node to look for a newer version now. The result updates below as it arrives.",
  },
];

/** The five Essentials rows, in their fixed order. A fresh array each call. */
export function essentialRows(): EssentialRow[] {
  return ESSENTIALS.map((r) => ({ ...r }));
}

// ---------------------------------------------------------------- loopback-only rule

/** True when the sharing controls must be disabled: the node says this session is not loopback. */
export function sharingDisabled(status: Status | null | undefined): boolean {
  return status?.auth?.loopback === false;
}

/** A `403 loopback_only` answer from the node. */
export function isLoopbackOnlyError(err: unknown): boolean {
  return isApiError(err) && err.code === "loopback_only";
}

/** Shown under the LAN toggle when DASHBOARD_LAN in the node's environment file pins LAN sharing. */
export const LAN_LOCKED_TEXT = "Pinned by DASHBOARD_LAN in the node's environment file — change it there.";

/** A `409 settings_locked` answer from the node (a PATCH of a setting the environment file pins). */
export function isSettingsLockedError(err: unknown): boolean {
  return isApiError(err) && err.code === "settings_locked";
}

/** Plain text for an action failure. A loopback-only refusal reads as the disabled-state text. */
export function problemText(err: unknown): string {
  if (isLoopbackOnlyError(err)) return LOOPBACK_ONLY_TEXT;
  if (isSettingsLockedError(err)) return LAN_LOCKED_TEXT;
  if (isApiError(err)) {
    if (err.code === "network") return "The node could not be reached. Check the connection and try again.";
    return err.problem.detail || err.problem.title || "The node refused the request.";
  }
  return err instanceof Error && err.message ? err.message : "Something went wrong.";
}

/** An alias can be 64 characters; the inline confirm keeps the question readable (same bound as Forget). */
export const RESTART_NAME_MAX = 40;

/** The destructive confirm label: "Restart {alias ?? label}". */
export function restartConfirmText(status: Status | null | undefined): string {
  const n = status?.node;
  const name = (n?.alias?.trim() || n?.label || "this node").slice(0, RESTART_NAME_MAX);
  return `Restart ${name}`;
}

/** The value to PATCH for the node name: trimmed, or `null` to clear. */
export function aliasPatchValue(draft: string): string | null {
  const t = draft.trim();
  return t === "" ? null : t;
}

/** LAN sharing was changed but the running daemon still has the old bind: a restart is pending. */
export function lanRestartPending(status: Status | null | undefined, settings: Settings | null | undefined): boolean {
  if (!status || !settings) return false;
  return settings.lan_enabled !== status.bind.lan;
}

export interface LanNote {
  kind: "locked" | "restart";
  text: string;
}

/**
 * The sentence under the LAN toggle. It is derived from the settings the node last returned, never
 * from the value that was requested, so the toggle and the sentence cannot disagree.
 * `lockedSeen`: a PATCH answered 409 `settings_locked` before the settings were refetched.
 */
export function lanNote(status: Status | null | undefined, settings: Settings | null | undefined, lockedSeen = false): LanNote | null {
  if (settings?.lan_locked || lockedSeen) return { kind: "locked", text: LAN_LOCKED_TEXT };
  if (settings && lanRestartPending(status, settings)) {
    return { kind: "restart", text: `LAN sharing ${settings.lan_enabled ? "on" : "off"}. Restart the node to apply.` };
  }
  return null;
}

export type OfferState = "offered" | "accepted" | "dismissed" | null;

/** Plain label for the standard-tier offer state. */
export function offerLabel(offer: OfferState | undefined): string {
  switch (offer) {
    case "offered":
      return "offered";
    case "accepted":
      return "accepted";
    case "dismissed":
      return "dismissed";
    default:
      return "not offered on this node";
  }
}

/** Which offer buttons make sense: accept while not accepted, dismiss only while offered. */
export function offerActions(offer: OfferState | undefined): { accept: boolean; dismiss: boolean } {
  return { accept: offer === "offered" || offer === "dismissed", dismiss: offer === "offered" };
}

// ---------------------------------------------------------------- update block

export interface FactRow {
  label: string;
  /** A string, or several lines stacked (URL lists). */
  value: string | string[];
  /** When set, the view renders a copy button for this text next to the value. */
  copy?: string;
}

const words = (s: string | null | undefined): string => (s ?? "unknown").replace(/_/g, " ");

/** Local date-time text for an ISO timestamp, or the raw text when it does not parse. */
export function formatWhen(iso: string | null | undefined): string {
  if (!iso) return "never";
  const t = Date.parse(iso);
  if (Number.isNaN(t)) return iso;
  return new Date(t).toLocaleString();
}

/** The update fields shown under "Check for updates" (live from /status). */
export function updateRows(status: Status | null | undefined): FactRow[] {
  const u = status?.update;
  if (!u) return [{ label: "State", value: "waiting for the node" }];
  const rows: FactRow[] = [
    { label: "State", value: words(u.state) },
    { label: "Last check", value: formatWhen(u.last_check) },
    { label: "Last result", value: u.last_result ? words(u.last_result) : "none yet" },
  ];
  if (u.pending_version) rows.push({ label: "Pending version", value: u.pending_version });
  return rows;
}

// ---------------------------------------------------------------- layer pages

export interface LayerCtx {
  /** Registry records (for the Relay layer's gateway state); null while loading. */
  services?: readonly ServiceRecord[] | null;
  /** Count of open proposals (for Repairs); null while loading. */
  openProposals?: number | null;
}

const WAITING: FactRow[] = [{ label: "Status", value: "waiting for the node" }];

const onOff = (b: boolean | undefined): string => (b ? "on" : "off");
const yesNo = (b: boolean | undefined): string => (b ? "yes" : "no");
const bindLines = (status: Status): string | string[] => (status.bind?.urls?.length ? [...status.bind.urls] : "nothing bound");
const shortHash = (h: string): string => (h.length > 16 ? `${h.slice(0, 16)}…` : h);

/** The status fields layer `n` owns today, from /status. Never throws; null status yields a waiting row. */
export function layerStatusRows(n: number, status: Status | null | undefined, ctx: LayerCtx = {}): FactRow[] {
  switch (n) {
    case 5:
    case 8:
      return [{ label: "Status", value: `arrives with Layer ${n}` }];
    case 9: {
      const c = ctx.openProposals;
      return [{ label: "Open proposals", value: c === null || c === undefined ? "loading" : String(c) }];
    }
    case 7: {
      if (!ctx.services) return [{ label: "Gateway service", value: "loading" }];
      const g = ctx.services.find((s) => s.id === "gateway");
      if (!g) return [{ label: "Gateway service", value: "not reported by this node" }];
      return [
        { label: "Gateway service", value: g.state },
        { label: "Reason", value: g.reason || "none" },
      ];
    }
  }
  if (!status) return WAITING.map((r) => ({ ...r }));
  switch (n) {
    case 0: {
      const kb = status.kb;
      if (!kb) return [{ label: "Knowledge base", value: "not present on this node" }];
      const rows: FactRow[] = [];
      if (kb.sha256) rows.push({ label: "Snapshot sha256", value: shortHash(kb.sha256), copy: kb.sha256 });
      rows.push({ label: "Entries", value: kb.entries === undefined ? "unknown" : String(kb.entries) });
      return rows;
    }
    case 1: {
      const rows: FactRow[] = [{ label: "Version", value: status.node.version ?? "unknown" }];
      for (const r of updateRows(status)) rows.push({ ...r, label: `Update ${r.label.toLowerCase()}` });
      rows.push({ label: "Clock fault", value: yesNo(status.update.clock_fault) });
      if (status.update.floor) rows.push({ label: "Version floor", value: status.update.floor });
      rows.push({ label: "Listening on", value: bindLines(status) });
      rows.push({ label: "LAN bound now", value: onOff(status.bind.lan) });
      return rows;
    }
    case 2:
      return [
        { label: "Node label", value: status.node.label ?? "not set" },
        status.node.bech32
          ? { label: "Address (bech32)", value: status.node.bech32, copy: status.node.bech32 }
          : { label: "Address (bech32)", value: "not set" },
        status.node.node_id
          ? { label: "Node id (hex)", value: status.node.node_id, copy: status.node.node_id }
          : { label: "Node id (hex)", value: "not set" },
      ];
    case 3: {
      const i = status.intelligence;
      return [
        { label: "Agent", value: i.agent ?? "unknown" },
        { label: "Model server", value: i.model_server ?? "unknown" },
        { label: "Agent mode", value: i.agent_state ?? "unknown" },
        { label: "Model", value: i.model_id ?? "none" },
      ];
    }
    case 4:
      return [
        { label: "Listening on", value: bindLines(status) },
        { label: "Loopback session", value: yesNo(status.auth.loopback) },
        { label: "Paired session", value: yesNo(status.auth.paired) },
        { label: "LAN sharing (configured)", value: onOff(status.lan_enabled) },
      ];
    case 6: {
      const rows: FactRow[] = [{ label: "Transport state", value: status.transport_state ?? "unknown" }];
      const c = status.health.components?.transport;
      if (c) rows.push({ label: "Transport component", value: c });
      return rows;
    }
    default:
      return [{ label: "Status", value: "no such layer" }];
  }
}

/** The staged prompt for "Ask the agent about {title}". */
export function layerAskPrompt(
  n: number,
  title: string,
  status: Status | null,
): { text: string; from: string; goal: PromptGoal } {
  const prefix = `About Layer ${n} (${title}): what is its state on this node and what should I configure?`;
  return { text: questionPrompt(prefix, { kind: "node" }, { status }), from: title, goal: "check" };
}
