// Pure helpers for the Devices views (spec 6.3, 7.1). No React: unit-tested in the node environment.
// Nothing here knows about a specific board or phone. Identification is the registry's job and
// the agent's; these helpers only turn registry fields into plain words and prepared prompts.
import type { DotState } from "../components/status-dot";
import type { DeviceKind, DeviceRecord, DeviceState, DeviceTransport, ServiceRecord, Status } from "./api/types";
import { buildPrompt, sanitizeLine, type PromptGoal } from "./prompts";

// ---------------------------------------------------------------- state and dots

/** connected is live, needs_setup is unset, error is error, disconnected is hollow. A lost node link hollows everything. */
export function deviceDot(state: DeviceState, disconnected: boolean): DotState {
  if (disconnected) return "unknown";
  switch (state) {
    case "connected":
      return "live";
    case "needs_setup":
      return "unset";
    case "error":
      return "error";
    default:
      return "unknown";
  }
}

/** The visible text that always accompanies a device dot. */
export function stateLabel(state: DeviceState, disconnected = false): string {
  if (disconnected) return "unknown";
  switch (state) {
    case "connected":
      return "connected";
    case "needs_setup":
      return "needs setup";
    case "error":
      return "error";
    default:
      return "disconnected";
  }
}

// ---------------------------------------------------------------- names and kinds

/** The user's name when there is one, else the detected name. A blank alias counts as none. */
export function displayName(d: Pick<DeviceRecord, "alias" | "name">): string {
  return d.alias !== null && d.alias !== undefined && d.alias.trim() !== "" ? d.alias : d.name;
}

/**
 * What to send as `alias` for the text in the rename field: empty, or the detected name itself,
 * clears the alias (null). Anything else is the trimmed text.
 */
export function aliasFromDraft(draft: string, d: Pick<DeviceRecord, "name">): string | null {
  const t = draft.trim();
  return t === "" || t === d.name ? null : t;
}

export const KIND_ORDER: readonly DeviceKind[] = ["radio", "bluetooth", "companion", "node", "analog", "unknown"];

export const KIND_TITLE: Record<DeviceKind, string> = {
  radio: "LoRa boards",
  bluetooth: "Bluetooth",
  companion: "Your phones & laptops",
  node: "Your other nodes",
  analog: "Handheld radios",
  unknown: "Unidentified",
};

/** Singular plain word for one device of this kind. */
export const KIND_WORD: Record<DeviceKind, string> = {
  radio: "LoRa board",
  bluetooth: "Bluetooth",
  companion: "Phone or laptop",
  node: "111 node",
  analog: "Handheld radio",
  unknown: "Unidentified",
};

export const TRANSPORT_WORD: Record<DeviceTransport, string> = {
  "usb-serial": "USB serial",
  spi: "SPI",
  bluetooth: "Bluetooth",
  wifi: "Wi-Fi",
  lan: "Local network",
  tcp: "TCP",
  audio: "Audio cable",
  unknown: "unknown",
};

export function kindWord(kind: string): string {
  return KIND_WORD[kind as DeviceKind] ?? KIND_WORD.unknown;
}

export function transportWord(transport: string): string {
  return TRANSPORT_WORD[transport as DeviceTransport] ?? "unknown";
}

/** "LoRa board · USB serial"; when both say the same thing ("Bluetooth · Bluetooth") it is said once. */
export function kindTransport(d: Pick<DeviceRecord, "kind" | "transport">): string {
  const kind = kindWord(d.kind);
  const transport = transportWord(d.transport);
  return kind === transport ? kind : `${kind} · ${transport}`;
}

function kindOf(d: Pick<DeviceRecord, "kind">): DeviceKind {
  return KIND_ORDER.includes(d.kind) ? d.kind : "unknown";
}

export interface DeviceGroup {
  kind: DeviceKind;
  title: string;
  devices: DeviceRecord[];
}

/**
 * Non-empty groups in the fixed kind order. Inside a group the registry order is kept, except that
 * devices which are not present sink to the bottom. A kind this UI does not know lands in Unidentified.
 */
export function groupByKind(devices: readonly DeviceRecord[]): DeviceGroup[] {
  const out: DeviceGroup[] = [];
  for (const kind of KIND_ORDER) {
    const inKind = devices.filter((d) => kindOf(d) === kind);
    if (inKind.length === 0) continue;
    const present = inKind.filter((d) => d.state !== "disconnected");
    const gone = inKind.filter((d) => d.state === "disconnected");
    out.push({ kind, title: KIND_TITLE[kind], devices: [...present, ...gone] });
  }
  return out;
}

// ---------------------------------------------------------------- identification and driver

/** "identified" (exact match), "likely" (a table match names the serial chip, not the board), else "unknown". */
export function confidenceLabel(d: Pick<DeviceRecord, "match">): "identified" | "likely" | "unknown" {
  switch (d.match?.confidence) {
    case "exact":
      return "identified";
    case "likely":
      return "likely";
    default:
      return "unknown";
  }
}

export const NOT_DRIVEN = "nothing yet";

/**
 * What drives the device right now, from `driver.stack`. A paired dashboard session carries no driver
 * on older records, and is by definition driven by the session.
 */
export function drivenBy(d: Pick<DeviceRecord, "driver" | "kind" | "capabilities">): string {
  switch (d.driver?.stack) {
    case "reticulum":
      return "Mesh transport (RNode)";
    case "meshtastic":
      return "Meshtastic";
    case "meshcore":
      return "MeshCore";
    case "dashboard":
      return "Dashboard session";
    default:
      return d.kind === "companion" && !d.driver && d.capabilities?.includes("dashboard-session") ? "Dashboard session" : NOT_DRIVEN;
  }
}

export const FIRMWARE_UNKNOWN = "unknown — identification needs your consent, arrives with Layer 6";

const FIRMWARE_FAMILY: Record<string, string> = {
  rnode: "RNode",
  meshtastic: "Meshtastic",
  meshcore: "MeshCore",
  kiss: "KISS modem",
  none: "no firmware installed",
};

const FIRMWARE_SOURCE: Record<string, string> = { probe: "a probe", user: "you", agent: "the agent" };

export function firmwareText(d: Pick<DeviceRecord, "firmware">): string {
  const f = d.firmware;
  if (!f || f.family === "unknown") return FIRMWARE_UNKNOWN;
  const family = FIRMWARE_FAMILY[f.family] ?? f.family;
  const version = f.version ? ` ${f.version}` : "";
  const source = FIRMWARE_SOURCE[f.source];
  return `${family}${version}${source ? ` (reported by ${source})` : ""}`;
}

/** The firmware row only means something for boards and unidentified hardware, or when the record carries one. */
export function showsFirmware(d: Pick<DeviceRecord, "kind" | "firmware">): boolean {
  return !!d.firmware || d.kind === "radio" || d.kind === "unknown";
}

export function roleText(role: DeviceRecord["role"] | undefined): string | null {
  switch (role) {
    case "controller":
      return "controls this node";
    case "terminal":
      return "light terminal of this node";
    case "relay":
      return "relays for this node";
    case "node":
      return "a node of its own";
    default:
      return null;
  }
}

// ---------------------------------------------------------------- identity rows

export interface IdentityRow {
  key: string;
  label: string;
  value: string;
  copy: boolean;
}

const IDENTITY: ReadonlyArray<readonly [key: string, label: string, copy: boolean]> = [
  ["vendor_id", "Vendor id", false],
  ["product_id", "Product id", false],
  ["serial", "Serial", true],
  ["port", "Port", true],
  ["by_id", "Port (by-id)", true],
  ["mac", "MAC address", true],
  ["hci", "Adapter", false],
  ["ip", "IP address", true],
  ["user_agent", "Browser", false],
];

const MAX_ID_CHARS = 300;

function asText(v: unknown): string | null {
  if (v === null || v === undefined) return null;
  let s: string;
  if (typeof v === "string") s = v;
  else if (typeof v === "number" || typeof v === "boolean") s = String(v);
  else {
    try {
      s = JSON.stringify(v) ?? "";
    } catch {
      return null;
    }
  }
  s = sanitizeLine(s);
  return s === "" ? null : s.slice(0, MAX_ID_CHARS);
}

/** `identity` as labelled rows, known keys first in a fixed order. Values are untrusted text: never markup. */
export function identityRows(d: Pick<DeviceRecord, "identity">): IdentityRow[] {
  const identity = d.identity ?? {};
  const rows: IdentityRow[] = [];
  const seen = new Set<string>();
  for (const [key, label, copy] of IDENTITY) {
    const value = asText(identity[key]);
    seen.add(key);
    if (value !== null) rows.push({ key, label, value, copy });
  }
  for (const key of Object.keys(identity)) {
    if (seen.has(key)) continue;
    const value = asText(identity[key]);
    if (value !== null) rows.push({ key, label: sanitizeLine(key).replace(/_/g, " ").slice(0, 40), value, copy: false });
  }
  return rows;
}

// ---------------------------------------------------------------- time

const MIN = 60;
const HOUR = 3600;
const DAY = 86_400;

function secondsSince(iso: string | undefined | null, nowMs: number): number | null {
  const t = Date.parse(iso ?? "");
  return Number.isFinite(t) ? Math.max(0, Math.floor((nowMs - t) / 1000)) : null;
}

/** "just now", "42s ago", "5m ago", "3h ago", "2d ago"; "unknown" for a missing or broken timestamp. */
export function agoText(iso: string | undefined | null, nowMs: number): string {
  const s = secondsSince(iso, nowMs);
  if (s === null) return "unknown";
  if (s < 10) return "just now";
  if (s < MIN) return `${s}s ago`;
  if (s < HOUR) return `${Math.floor(s / MIN)}m ago`;
  if (s < DAY) return `${Math.floor(s / HOUR)}h ago`;
  return `${Math.floor(s / DAY)}d ago`;
}

/** "a few seconds", "5m", "3h", "2d": how long ago something started, without the word "ago". */
export function durationText(iso: string | undefined | null, nowMs: number): string {
  const s = secondsSince(iso, nowMs);
  if (s === null) return "an unknown time";
  if (s < 10) return "a few seconds";
  if (s < MIN) return `${s}s`;
  if (s < HOUR) return `${Math.floor(s / MIN)}m`;
  if (s < DAY) return `${Math.floor(s / HOUR)}h`;
  return `${Math.floor(s / DAY)}d`;
}

/**
 * Header line of the detail page, read after the state badge: "for 4m", "last seen 3h ago".
 * While the node link is down the state is only the last known one.
 */
export function presenceText(d: Pick<DeviceRecord, "state" | "since" | "last_seen">, nowMs: number, nodeDisconnected = false): string {
  if (nodeDisconnected) return `last known state, last seen ${agoText(d.last_seen, nowMs)}`;
  if (d.state === "disconnected") return `last seen ${agoText(d.last_seen, nowMs)}`;
  return `for ${durationText(d.since, nowMs)}`;
}

// ---------------------------------------------------------------- summary

export interface DeviceSummary {
  total: number;
  connected: number;
  needsSetup: number;
  error: number;
  disconnected: number;
  sentence: string;
}

export function summary(devices: readonly Pick<DeviceRecord, "state">[]): DeviceSummary {
  const count = (s: DeviceState) => devices.filter((d) => d.state === s).length;
  const connected = count("connected");
  const needsSetup = count("needs_setup");
  const error = count("error");
  const disconnected = count("disconnected");
  const total = devices.length;
  const noun = total === 1 ? "device" : "devices";
  let sentence: string;
  if (total === 0) sentence = "Nothing connected yet.";
  else if (connected === total) sentence = total === 1 ? "1 device, connected." : `${total} devices, all connected.`;
  else {
    const parts: string[] = [];
    if (connected) parts.push(`${connected} connected`);
    if (needsSetup) parts.push(`${needsSetup} ${needsSetup === 1 ? "needs" : "need"} setup`);
    if (error) parts.push(`${error} ${error === 1 ? "has an error" : "have errors"}`);
    if (disconnected) parts.push(`${disconnected} disconnected`);
    sentence = `${total} ${noun}: ${parts.join(", ")}.`;
  }
  return { total, connected, needsSetup, error, disconnected, sentence };
}

// ---------------------------------------------------------------- prompts (spec 8: one builder, only the context differs)

/** The agent prompt for the whole registry. `facts` adds every device's raw detection facts (bounded by the builder). */
export function registryPrompt(a: {
  devices: readonly DeviceRecord[];
  services: readonly ServiceRecord[];
  status: Status | null;
  hint?: string;
  facts?: boolean;
}): string {
  return buildPrompt(
    "connect",
    { kind: "registry", devices: [...a.devices], services: [...a.services] },
    { status: a.status, hint: a.hint, rawFacts: a.facts ? a.devices.map((d) => d.detail) : undefined },
  );
}

export interface DeviceAgentAction {
  goal: PromptGoal;
  /** Button text on the detail page. */
  label: string;
}

/** An error is fixed, an unbound device is set up, anything else is asked about. */
export function deviceAgentAction(d: Pick<DeviceRecord, "state">): DeviceAgentAction {
  if (d.state === "error") return { goal: "fix", label: "Let the agent handle it" };
  if (d.state === "needs_setup") return { goal: "setup", label: "Let the agent handle it" };
  return { goal: "check", label: "Ask the agent about this device" };
}

/** Device names are untrusted strings; they are cleaned and bounded before they go into prompt text. */
function promptName(d: Pick<DeviceRecord, "alias" | "name">): string {
  return sanitizeLine(displayName(d)).slice(0, 80);
}

export function devicePrompt(a: {
  device: DeviceRecord;
  services: readonly ServiceRecord[];
  status: Status | null;
}): { text: string; goal: PromptGoal } {
  const { device } = a;
  const { goal } = deviceAgentAction(device);
  const service = device.service_id ? (a.services.find((s) => s.id === device.service_id) ?? null) : null;
  const body = buildPrompt(goal, { kind: "device", device, service }, { status: a.status, rawFacts: device.detail });
  // The check goal sentence is about the whole node; say which device the question is about.
  const text = goal === "check" ? `About the device ${promptName(device)}: what is it, is it working, what can it do?\n\n${body}` : body;
  return { text, goal };
}

/** Text of the inline confirm for Forget. The Button appends the question mark. */
export function forgetConfirm(d: Pick<DeviceRecord, "kind" | "alias" | "name">): string {
  const name = displayName(d).slice(0, 40);
  switch (d.kind) {
    case "companion":
      return `Forget ${name}? If this is the phone you are using right now, you will be logged out. Go ahead`;
    case "node":
      return `Forget ${name}? The link to that node is removed. Go ahead`;
    default:
      return `Forget ${name}? It shows up again if it is still plugged in, but its name is lost. Go ahead`;
  }
}

// ---------------------------------------------------------------- Add device: pairing help

export type PairingState = "unknown" | "off" | "pending" | "on";

export interface PairingHelp {
  state: PairingState;
  body: string;
  /** The command to run on the node for the pair code, only when pairing is possible. */
  command: string | null;
  /** Addresses a phone on the same network can open, nicest first. Empty unless `state` is "on". */
  urls: string[];
  /** The first address with the pair route on it, for the large display. */
  pairUrl: string | null;
  /** Route to the setting that turns LAN sharing on, when that is the fix. */
  settingsHref: string | null;
}

export function isLoopbackUrl(url: string): boolean {
  let host: string;
  try {
    host = new URL(url).hostname.toLowerCase();
  } catch {
    return false;
  }
  host = host.replace(/^\[|\]$/g, "");
  return host === "localhost" || host.endsWith(".localhost") || host === "::1" || /^127\./.test(host);
}

export const PAIR_COMMAND = "111 dashboard";

/** What the "Control this node" row shows, from `status.lan_enabled` (configured) and `status.bind` (actually bound). */
export function pairingHelp(status: Pick<Status, "lan_enabled" | "bind"> | null): PairingHelp {
  if (!status) {
    return { state: "unknown", body: "The pairing details appear as soon as the node answers.", command: null, urls: [], pairUrl: null, settingsHref: null };
  }
  if (!status.lan_enabled) {
    return {
      state: "off",
      body: "LAN sharing is off — turn it on in Settings → Sharing (from the node itself).",
      command: null,
      urls: [],
      pairUrl: null,
      settingsHref: "/settings",
    };
  }
  if (!status.bind?.lan) {
    return {
      state: "pending",
      body: "LAN sharing is switched on, but it only takes effect when the node restarts.",
      command: null,
      urls: [],
      pairUrl: null,
      settingsHref: "/settings",
    };
  }
  const urls = (status.bind.urls ?? []).filter((u) => !isLoopbackUrl(u));
  if (urls.length === 0) {
    return {
      state: "on",
      body: "No network address was found yet. Check that the node is connected to Wi-Fi or Ethernet.",
      command: null,
      urls: [],
      pairUrl: null,
      settingsHref: null,
    };
  }
  return {
    state: "on",
    body: "On the phone or laptop, join the same network and open one of these addresses. On the node, run the command below for the pair code.",
    command: PAIR_COMMAND,
    urls,
    pairUrl: `${urls[0]!.replace(/\/+$/, "")}/#/pair`,
    settingsHref: null,
  };
}
