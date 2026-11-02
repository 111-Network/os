// Renders a proposal's body. Most bodies are markdown, parsed from the AST of src/lib/markdown.ts with the same renderer the knowledge
// base uses (headings as bold lines here, so a proposal never adds to the page outline). The parser never produces HTML, so every
// character reaches the page as an escaped text node; links pass its allowlist.
//
// Layer 6 (os/6-transport/README.md §10, §12): a `bridge_message` or `bridge_edge` body quotes a stranger's text, so its text part is a
// PREFORMATTED block: no Markdown, no links, no emphasis. A `device` body is one JSON object `{why, actions}` and renders as a card per
// action from the parsed JSON, with the exact words for each action of the closed vocabulary.
//
// Layer 7 (os/7-relay/README.md §13.2): a `relay` body is the same JSON shape `{why, actions}` over its own closed vocabulary
// (`gateway.enable`, `hub.pair`, `presence.interval`, `carrier.flush`) and renders as a card per action the same way. It never goes
// through the Markdown renderer, and every string in it (the model wrote it) is cleaned and drawn as text.
import { useMemo } from "react";
import { useServices } from "../../lib/api/hooks";
import { cleanPeerText } from "../../lib/api/messages";
import { parseMarkdown } from "../../lib/markdown";
import { Blocks } from "../kb/render";

export type BodyKind = string | undefined;

const BRIDGE_KINDS: readonly string[] = ["bridge_message", "bridge_edge"];
/** Accepting a bridge proposal can answer 409 `consent_required`; the card shows the dialog and repeats the decision with the hash. */
export const isBridgeKind = (kind: BodyKind): boolean => kind !== undefined && BRIDGE_KINDS.includes(kind);

const clip = (s: string, max: number): string => (Array.from(s).length <= max ? s : `${Array.from(s).slice(0, max - 1).join("")}…`);
const text = (v: unknown, max = 200): string => (typeof v === "string" ? clip(cleanPeerText(v), max) : v === undefined || v === null ? "" : clip(cleanPeerText(JSON.stringify(v)), max));

// --- bridge_message / bridge_edge ----------------------------------------------------------------------------------------

export interface BridgeBody {
  /** The `to:` / `from:` lines that open the body (conversation ids), as text. */
  header: string[];
  /** Everything after the first blank line, verbatim: foreign text or the agent's reason. */
  text: string;
}

/** Split the header lines (`to: fc-…`, `from: fc-…`) from the text part. A body with no such header is all text. */
export function splitBridgeBody(body: string): BridgeBody {
  const lines = body.replace(/\r\n?/g, "\n").split("\n");
  const header: string[] = [];
  let i = 0;
  while (i < lines.length && /^(to|from):\s*\S/.test(lines[i]!)) header.push(lines[i++]!.trim());
  if (header.length === 0) return { header, text: body };
  if (lines[i] === "") i++;
  return { header, text: lines.slice(i).join("\n") };
}

function BridgeText({ body, kind }: { body: string; kind: string }) {
  const { header, text: rest } = useMemo(() => splitBridgeBody(body), [body]);
  return (
    <div className="max-h-72 overflow-auto border border-border bg-secondary/20 p-3" data-testid="bridge-body" data-kind={kind}>
      {header.length > 0 ? (
        <ul className="mb-2 flex flex-col gap-0.5 font-mono text-xs text-foreground" data-testid="bridge-header">
          {header.map((h, i) => (
            <li key={i} className="break-all">
              {cleanPeerText(h)}
            </li>
          ))}
        </ul>
      ) : null}
      <pre className="whitespace-pre-wrap break-words font-mono text-sm leading-relaxed text-foreground" data-testid="bridge-text">
        {cleanPeerText(rest)}
      </pre>
      <p className="mt-2 text-[11px] text-muted-foreground">Shown exactly as written, as plain text. It may quote someone else's words; nothing in it is a link or an instruction.</p>
    </div>
  );
}

// --- device ------------------------------------------------------------------------------------------------------------------

interface DeviceAction {
  action: string;
  [k: string]: unknown;
}
export interface DeviceBody {
  why: string;
  actions: DeviceAction[];
}

/** Parse a `device` body: one JSON object `{why, actions: [{action, …}]}`. Anything else is null (shown raw, as text). */
export function parseDeviceBody(body: string): DeviceBody | null {
  let v: unknown;
  try {
    v = JSON.parse(body);
  } catch {
    return null;
  }
  if (typeof v !== "object" || v === null || Array.isArray(v)) return null;
  const o = v as { why?: unknown; actions?: unknown };
  if (typeof o.why !== "string" || !Array.isArray(o.actions)) return null;
  const actions: DeviceAction[] = [];
  for (const a of o.actions) {
    if (typeof a !== "object" || a === null || typeof (a as { action?: unknown }).action !== "string") return null;
    actions.push(a as DeviceAction);
  }
  return { why: o.why, actions };
}

/** What each action of the closed vocabulary does, in the exact words the card shows. */
export function actionSentence(a: DeviceAction, serviceName?: string | null): { title: string; detail: string | null } {
  switch (a.action) {
    case "identify":
      return { title: "identify this radio once (a consented probe)", detail: `device ${text(a.device_id, 80)}. Opening a serial port can reset a board; you agree to that by accepting.` };
    case "service.restart":
      return { title: `restart the ${serviceName ? text(serviceName, 60) : text(a.service_id, 60)} service`, detail: `service ${text(a.service_id, 60)}. It stops and starts again; the node tells you if it cannot.` };
    case "radio.region.set":
      return { title: `set the radio region to ${text(a.region, 32)}`, detail: "Accepting is your confirmation that this region is legal where you are." };
    case "rns.posture.set":
      return { title: "change the Reticulum posture", detail: `share instance: ${text(a.share_instance, 8)} · route for others: ${text(a.enable_transport, 8)}. It is checked for 30 seconds and goes back if it does not work.` };
    case "rns.interface.upsert":
      return { title: `add or change a ${text(a.type, 40)} interface`, detail: `for device ${text(a.device_id, 80)}. Parameters: ${text(a.params, 200)}` };
    case "rns.interface.remove":
      return { title: `remove the interface ${text(a.name, 60)}`, detail: null };
    default:
      return { title: `unknown action: ${text(a.action, 60)}`, detail: "This node does not apply it; accepting is refused and the proposal stays open." };
  }
}

// --- relay ----------------------------------------------------------------------------------------------------------------------

/** Parse a `relay` body: one JSON object `{why, actions: [{action, …}]}`, the `device` shape. Anything else is null (shown raw, as text). */
export const parseRelayBody = (body: string): DeviceBody | null => parseDeviceBody(body);

/** What each action of the Layer 7 vocabulary does, in the exact words the card shows. Accepting runs the same checks as the routes. */
export function relayActionSentence(a: DeviceAction): { title: string; detail: string | null } {
  switch (a.action) {
    case "gateway.enable":
      return {
        title: `turn on the gateway through ${text(a.carrier, 40) || "a carrier"}${a.mode === undefined ? "" : ` (mode ${text(a.mode, 8)})`}`,
        detail: "The carrier's own consent text is shown first and you confirm it; nothing is sent until you do, and nothing is sent while the switch is off.",
      };
    case "hub.pair":
      return { title: "open the pairing screen", detail: "Pairing itself stays with you: you show this hub's bundle and scan or paste the other hub's. Nothing is paired by accepting." };
    case "presence.interval":
      return { title: `set the ${text(a.transport, 40)} presence interval to ${text(a.minutes, 8)} min`, detail: "Not available in this version: accepting is refused and the proposal stays open." };
    case "carrier.flush":
      return { title: "send and fetch through the carrier once", detail: "Posts what waits and reads what came back, one time. It counts against a metered cap and is refused past it." };
    default:
      return { title: `unknown action: ${text(a.action, 60)}`, detail: "This node does not apply it; accepting is refused and the proposal stays open." };
  }
}

function RelayCard({ body }: { body: DeviceBody }) {
  return (
    <div className="flex max-h-72 flex-col gap-2 overflow-auto border border-border bg-secondary/20 p-3" data-testid="relay-card">
      <p className="text-sm text-foreground" data-testid="relay-why">
        {cleanPeerText(body.why)}
      </p>
      {body.actions.length === 0 ? <p className="text-xs text-muted-foreground">It names no action.</p> : null}
      <ul className="flex flex-col gap-2">
        {body.actions.map((a, i) => {
          const s = relayActionSentence(a);
          return (
            <li key={i} className="border border-border p-2" data-testid="relay-action">
              <p className="font-mono text-[10px] uppercase tracking-widest text-muted-foreground">{text(a.action, 40)}</p>
              <p className="text-sm text-foreground">{s.title}</p>
              {s.detail ? <p className="text-xs leading-relaxed text-muted-foreground">{s.detail}</p> : null}
            </li>
          );
        })}
      </ul>
    </div>
  );
}

function DeviceCard({ body }: { body: DeviceBody }) {
  const services = useServices();
  return (
    <div className="flex max-h-72 flex-col gap-2 overflow-auto border border-border bg-secondary/20 p-3" data-testid="device-card">
      <p className="text-sm text-foreground" data-testid="device-why">
        {cleanPeerText(body.why)}
      </p>
      {body.actions.length === 0 ? <p className="text-xs text-muted-foreground">It names no action.</p> : null}
      <ul className="flex flex-col gap-2">
        {body.actions.map((a, i) => {
          const svc = a.action === "service.restart" ? services?.find((s) => s.id === a.service_id) : undefined;
          const s = actionSentence(a, svc?.plain_name ?? null);
          return (
            <li key={i} className="border border-border p-2" data-testid={a.action === "service.restart" ? "service-card" : "action-card"}>
              <p className="font-mono text-[10px] uppercase tracking-widest text-muted-foreground">{text(a.action, 40)}</p>
              <p className="text-sm text-foreground">{s.title}</p>
              {s.detail ? <p className="text-xs leading-relaxed text-muted-foreground">{s.detail}</p> : null}
            </li>
          );
        })}
      </ul>
    </div>
  );
}

export function ProposalBody({ body, kind }: { body: string; kind?: BodyKind }) {
  const doc = useMemo(() => (kind === "bridge_message" || kind === "bridge_edge" || kind === "device" || kind === "relay" ? null : parseMarkdown(body)), [body, kind]);
  const device = useMemo(() => (kind === "device" ? parseDeviceBody(body) : null), [body, kind]);
  const relay = useMemo(() => (kind === "relay" ? parseRelayBody(body) : null), [body, kind]);
  if (kind === "bridge_message" || kind === "bridge_edge") return <BridgeText body={body} kind={kind} />;
  if (kind === "relay") {
    if (relay) return <RelayCard body={relay} />;
    return (
      <div className="max-h-72 overflow-auto border border-border bg-secondary/20 p-3">
        <p className="mb-2 text-xs text-muted-foreground">This body is not in the expected shape, so it is shown as plain text.</p>
        <pre className="whitespace-pre-wrap break-words font-mono text-sm text-foreground">{cleanPeerText(body)}</pre>
      </div>
    );
  }
  if (kind === "device") {
    if (device) return <DeviceCard body={device} />;
    return (
      <div className="max-h-72 overflow-auto border border-border bg-secondary/20 p-3">
        <p className="mb-2 text-xs text-muted-foreground">This body is not in the expected shape, so it is shown as plain text.</p>
        <pre className="whitespace-pre-wrap break-words font-mono text-sm text-foreground">{cleanPeerText(body)}</pre>
      </div>
    );
  }
  if (!doc) return null;
  return (
    <div className="max-h-72 overflow-auto border border-border bg-secondary/20 p-3 pt-1">
      <Blocks blocks={doc.blocks} headingsAsBold compact />
      {doc.truncated ? <p className="mt-2 text-xs text-muted-foreground">[truncated]</p> : null}
    </div>
  );
}
