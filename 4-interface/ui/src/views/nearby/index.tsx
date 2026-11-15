// Nearby (os/5-messaging/README.md section 14 and 16, interface README 6.4): the nodes a Meshtastic board has heard, grouped by
// source ("heard via Meshtastic on <board>"). Everything a node reports is an untrusted RF-origin string: it is drawn as text
// only (cleaned and cut), never as a link, and it never goes into a prompt. A Meshtastic node is not a contact. Layer 7 (os/7-relay/README.md
// section 10) adds the `111` source, the evidence and age behind each of its rows, and the SVG radar with its three buttons.
import { useEffect, useMemo, useState, useSyncExternalStore } from "react";
import { redirectIfUnauthenticated } from "../../app/AuthGate";
import { hrefTo } from "../../app/location";
import { useNow } from "../../app/hooks";
import { Badge } from "../../components/badge";
import { Button, ButtonLink } from "../../components/button";
import { Empty } from "../../components/empty";
import { Panel } from "../../components/panel";
import { SectionHeader } from "../../components/section-header";
import { apiStore, getRelay } from "../../lib/api";
import { useApiMeta, useConnection, useDevices, useFabric, useNearby, usePanels, useServices, useStatus } from "../../lib/api/hooks";
import { ageLabel, heardAgoNow, isOneOneOneSource, nodeEvidence } from "../../lib/api/nearby";
import { panelFor } from "../../lib/api/panels";
import type { NearbyNode, NearbySource, RegistryRow } from "../../lib/api/types";
import { Adv } from "../../lib/mode";
import { NEARBY_QUESTION, questionPrompt } from "../../lib/prompts";
import { AgentButton, useAgentReachable } from "../devices/agent-button";
import { DmAction } from "./dm-action";
import { RouteLink } from "../services/shared";
import {
  createRadarActions,
  displayName,
  evidenceLine,
  GLYPH_KEY,
  heardSummary,
  heardLine,
  isMeshtasticSource,
  hopsText,
  keyText,
  nearbyState,
  plain,
  positionText,
  RADAR_LEGEND,
  registryLine,
  radarModel,
  shortName,
  signalText,
  sortedNodes,
  sourceHeading,
  utilizationText,
  type RadarNotice,
} from "./model";
import { nearbySentence, RadarSvg } from "./radar";

const TEXT = "font-mono text-xs leading-relaxed text-muted-foreground";
/** Text a radio supplied: its own direction, isolated from the page around it, wrapped wherever it must be. */
const RF_TEXT = "min-w-0 break-words [unicode-bidi:isolate]";

function NodeItem({ node, now, receivedAt, sourceKind }: { node: NearbyNode; now: number; receivedAt: number; sourceKind: string }) {
  const short = shortName(node);
  // A 111 row's age is this node's own: `heard_ago_s` as of the frame plus the time this page has held it (no timer on the node, no poll).
  const age = heardAgoNow(node, receivedAt, now);
  const evidence = nodeEvidence(node);
  const facts = [hopsText(node), signalText(node), utilizationText(node)].filter((x): x is string => x !== null);
  const position = positionText(node);
  const key = keyText(node);
  return (
    <li className="flex flex-col gap-1 px-4 py-3" data-testid="nearby-node">
      <div className="flex flex-wrap items-baseline gap-x-3 gap-y-1">
        <span dir="auto" className={`${RF_TEXT} font-mono text-sm text-foreground`}>
          {displayName(node)}
        </span>
        {short ? (
          <Badge dir="auto" className="[unicode-bidi:isolate]">
            {short}
          </Badge>
        ) : null}
        <span className="font-mono text-[10px] uppercase tracking-widest text-muted-foreground">{age !== null ? `heard ${ageLabel(age)}` : heardLine(node, now)}</span>
      </div>
      {evidence.length > 0 ? (
        <ul className={TEXT} data-testid="nearby-evidence">
          {evidence.map((e, i) => (
            <li key={`${e.transport}/${i}`} dir="auto" className={RF_TEXT}>
              {evidenceLine(e)}
            </li>
          ))}
        </ul>
      ) : null}
      {facts.length > 0 ? <p className={TEXT}>{facts.join(" · ")}</p> : null}
      {position ? <p className={TEXT}>position, as it reports it: {position}</p> : null}
      <DmAction node={node} sourceKind={sourceKind} />
      <Adv>
        <p dir="auto" className={`${RF_TEXT} font-mono text-[11px] leading-snug text-muted-foreground`}>
          id {plain(node.id, 24)} · number {Number.isFinite(node.num) ? node.num : "unknown"}
          {key ? ` · key ${key}` : ""}
        </p>
      </Adv>
    </li>
  );
}

function SourceList({ source, now, receivedAt }: { source: NearbySource; now: number; receivedAt: number }) {
  const devices = useDevices();
  const heading = sourceHeading(source, devices);
  const nodes = sortedNodes(source.nodes);
  return (
    <Panel title={isOneOneOneSource(source) ? "111" : source.kind === "reticulum" ? "reticulum" : "meshtastic"} suffix="nodes heard" flush>
      <div className="border-b border-border px-4 py-3">
        <h2 dir="auto" className={`${RF_TEXT} font-mono text-sm text-foreground`}>
          {heading}
        </h2>
        <p className={`mt-1 ${TEXT}`}>{nodes.length === 1 ? "1 node" : `${nodes.length} nodes`}</p>
      </div>
      {nodes.length === 0 ? (
        <p className={`px-4 py-3 ${TEXT}`}>{isOneOneOneSource(source) ? "No 111 node heard yet." : source.kind === "reticulum" ? "No Reticulum announce heard yet." : "This board has not heard another node yet."}</p>
      ) : (
        <ul className="divide-y divide-border">
          {nodes.map((n) => (
            <NodeItem key={`${source.id}/${n.id}`} node={n} now={now} receivedAt={receivedAt} sourceKind={source.kind} />
          ))}
        </ul>
      )}
    </Panel>
  );
}

/** The count line above the lists: Meshtastic boards, Reticulum announces and 111 nodes each in their own words, never merged. */
function summaryOf(sources: readonly NearbySource[]): string {
  const mesh = sources.filter(isMeshtasticSource);
  const count = (pick: (s: NearbySource) => boolean): number => sources.filter(pick).reduce((n, s) => n + s.nodes.length, 0);
  const ret = count((s) => s.kind === "reticulum");
  const one = count(isOneOneOneSource);
  const parts: string[] = [];
  if (mesh.length > 0 || (ret === 0 && one === 0)) parts.push(heardSummary(mesh));
  if (ret > 0) parts.push(`${ret} Reticulum announces heard.`);
  if (one > 0) parts.push(one === 1 ? "1 111 node heard." : `${one} 111 nodes heard.`);
  return parts.join(" ");
}

/** Registry answers as the node sends them: each row names the layer that answered (a hint is unverified); its age runs from when this page received the rows. */
export function RegistryList({ rows, hubId, now, receivedAt }: { rows: readonly RegistryRow[]; hubId: string | null; now: number; receivedAt: number }) {
  return (
    <ul className="flex flex-col gap-1" data-testid="radar-registry">
      {rows.map((r) => {
        const age = heardAgoNow({ heard_ago_s: r.heard_ago_s }, receivedAt, now);
        return (
          <li key={r.id} dir="auto" className={`${RF_TEXT} ${TEXT}`} data-testid="radar-registry-row">
            {registryLine(r, hubId, age !== null ? ageLabel(age) : null)}
          </li>
        );
      })}
    </ul>
  );
}

/** The radar and its buttons (section 10). The page's clock comes in as `now`; nothing here polls. */
function RadarPanel({ sources, now, receivedAt }: { sources: readonly NearbySource[] | null; now: number; receivedAt: number }) {
  const fabric = useFabric();
  const panels = usePanels();
  const model = useMemo(() => radarModel(sources, { own: null, receivedAtMs: receivedAt, nowMs: now }), [sources, receivedAt, now]);
  const actions = useMemo(() => createRadarActions(getRelay), []);
  const registry = useSyncExternalStore(
    apiStore.subscribe,
    () => apiStore.getState().registry,
    () => apiStore.getState().registry,
  );
  // The registry slice has its own receive time (the nearby stamp is another clock): a row ages from the frame or read that brought it.
  const registryAt = useMemo(() => Date.now(), [registry]);
  // One GET /registry on mount, no poll: it holds rows heard without a browse and the last browse's rows after a reload. A node without
  // Layer 7 answers 404; the store keeps the slice as it was (null) and the page shows nothing more.
  useEffect(() => {
    void apiStore.refreshRegistry();
  }, []);
  const [busy, setBusy] = useState<"look" | "find" | "hops" | null>(null);
  const [notice, setNotice] = useState<RadarNotice | null>(null);
  const hubId = fabric?.hub.enrolled && fabric.hub.node_id ? fabric.hub.node_id : null;
  const hasReticulum = (sources ?? []).some((s) => s.kind === "reticulum");
  const mapPanel = panelFor(panels, "meshtastic");
  const mapOpen = mapPanel !== null && mapPanel.state.enabled && mapPanel.state.listening;

  const press = async (which: "look" | "find" | "hops") => {
    setBusy(which);
    setNotice(null);
    const result = await actions[which]();
    setBusy(null);
    if (result.cause !== undefined && redirectIfUnauthenticated(result.cause)) return;
    setNotice(result);
  };

  return (
    <Panel title="radar" tint>
      <div className="flex flex-col items-center gap-3" data-testid="radar-panel">
        <RadarSvg model={model} />
        <p className="max-w-prose text-center font-mono text-xs leading-relaxed text-muted-foreground" data-testid="radar-legend">
          {RADAR_LEGEND}
        </p>
        <p className="max-w-prose text-center font-mono text-[11px] leading-relaxed text-muted-foreground" data-testid="radar-glyphs">
          {GLYPH_KEY}
        </p>
        {model.hidden > 0 ? (
          <p className="max-w-prose text-center font-mono text-[11px] leading-relaxed text-muted-foreground" data-testid="radar-hidden">
            {model.hidden === 1 ? "1 more node is" : `${model.hidden} more nodes are`} not drawn; the lists hold every node.
          </p>
        ) : null}
        {model.capped ? (
          <p className="max-w-prose text-center font-mono text-[11px] leading-relaxed text-muted-foreground">The last ring also holds nodes farther away; their rows say how far.</p>
        ) : null}
        <div className="flex flex-wrap items-center justify-center gap-3">
          <Button size="sm" loading={busy === "look"} disabled={busy !== null} onClick={() => void press("look")} data-testid="radar-look">
            Look for 111 nodes here
          </Button>
          {hubId !== null ? (
            <Button size="sm" loading={busy === "find"} disabled={busy !== null} onClick={() => void press("find")} data-testid="radar-find-hub">
              Find my hub
            </Button>
          ) : null}
          {hasReticulum ? (
            <Button size="sm" loading={busy === "hops"} disabled={busy !== null} onClick={() => void press("hops")} data-testid="radar-hops">
              Read Reticulum hop counts
            </Button>
          ) : null}
          {mapOpen ? (
            <RouteLink to="/services/meshtastic" className="inline-flex min-h-11 items-center font-mono text-xs" data-testid="radar-map">
              Open the Meshtastic map
            </RouteLink>
          ) : null}
        </div>
        {notice ? (
          <p role={notice.tone === "problem" ? "alert" : "status"} dir="auto" className={`max-w-prose text-center font-mono text-xs leading-relaxed [unicode-bidi:isolate] ${notice.tone === "problem" ? "text-destructive" : "text-foreground"}`} data-testid="radar-notice">
            {notice.text}
          </p>
        ) : null}
        {registry && registry.length > 0 ? <RegistryList rows={registry} hubId={hubId} now={now} receivedAt={registryAt} /> : null}
        {mapOpen ? <p className="max-w-prose text-center font-mono text-[11px] leading-relaxed text-muted-foreground">The map comes from the Meshtastic panel and is blank without internet. 111 nodes never share a position unless you do; this version has no position setting.</p> : null}
      </div>
    </Panel>
  );
}

export default function NearbyView() {
  const status = useStatus();
  const nearby = useNearby();
  const services = useServices();
  const { disconnected } = useConnection();
  const { ready } = useApiMeta();
  const reachable = useAgentReachable();
  const now = useNow(true, 15_000);
  // The store keeps no receive time, so this page stamps each snapshot it is handed: an age is `heard_ago_s` plus the time this page has held it.
  const receivedAt = useMemo(() => Date.now(), [nearby]);
  const service = services?.find((s) => s.id === "meshtastic") ?? null;
  const state = nearbyState(nearby, service, disconnected, ready);
  const sentence = nearbySentence(status?.transport_state);

  const action =
    state.kind === "no_board" ? (
      <div className="flex flex-wrap items-center gap-3">
        <ButtonLink href={hrefTo("/services/meshtastic")} size="sm">
          Open the Meshtastic page
        </ButtonLink>
        <ButtonLink href={hrefTo("/devices")} size="sm">
          Open Devices
        </ButtonLink>
      </div>
    ) : state.kind === "unbound" || state.kind === "starting" || state.kind === "error" ? (
      <ButtonLink href={hrefTo("/services/meshtastic")} size="sm">
        Open the Meshtastic page
      </ButtonLink>
    ) : undefined;

  return (
    <div className="flex flex-col gap-6">
      <SectionHeader kicker="nearby" title="Nearby" level={1} />
      <p className="max-w-prose font-mono text-sm leading-relaxed text-foreground">{sentence}</p>

      {state.kind === "list" && nearby ? (
        <section aria-labelledby="nearby-lists" className="flex flex-col gap-3" data-testid="nearby-lists">
          <h2 id="nearby-lists" className="font-mono text-[10px] uppercase tracking-widest text-muted-foreground">
            heard by this node
          </h2>
          <p className="font-mono text-sm leading-relaxed text-foreground" data-testid="nearby-summary">{summaryOf(nearby)}</p>
          {state.text ? (
            <p role="status" className="max-w-prose border border-dashed border-border p-3 font-mono text-xs leading-relaxed text-foreground" data-testid="nearby-stale">
              {state.text}
            </p>
          ) : null}
          {nearby.map((s) => (
            <SourceList key={s.id} source={s} now={now} receivedAt={receivedAt} />
          ))}
          <p className={`max-w-prose ${TEXT}`}>
            Names, positions and times are what each node says about itself over the air. They are shown as plain text and are never links. A time comes from the node&apos;s own clock, so it is a hint, not proof that
            it is in range now. A Meshtastic node is not a contact: it is a different network with its own identities. Where it is offered, a DM opens as a Meshtastic conversation in Messages (readable, not 111-sealed).
          </p>
        </section>
      ) : (
        <Empty
          kicker={
            state.kind === "loading"
              ? "loading"
              : state.kind === "error"
                ? "problem"
                : state.kind === "starting"
                  ? "starting"
                  : state.kind === "no_board" || state.kind === "unbound"
                    ? "no board"
                    : state.kind === "unavailable"
                      ? "unavailable"
                      : "nobody heard"
          }
          title={
            state.kind === "no_board"
              ? "No Meshtastic board"
              : state.kind === "unbound"
                ? "A board is waiting to be set up"
                : state.kind === "starting"
                  ? "Meshtastic is starting"
                  : state.kind === "error"
                    ? "Meshtastic has a problem"
                    : state.kind === "empty"
                      ? "No other Meshtastic nodes heard yet"
                      : state.kind === "unavailable"
                        ? "Not reported"
                        : "Reading what this node hears"
          }
          {...(action ? { action } : {})}
        >
          {state.text}
        </Empty>
      )}

      <RadarPanel sources={nearby} now={now} receivedAt={receivedAt} />

      <div className="flex flex-wrap items-center gap-3">
        <AgentButton
          label="Ask the agent who is nearby"
          from="Nearby"
          goal="check"
          reachable={reachable}
          prompt={() => questionPrompt(NEARBY_QUESTION, { kind: "node" }, { status })}
        />
        {service ? (
          <RouteLink to="/services/meshtastic" className="inline-flex min-h-11 items-center font-mono text-xs">
            Meshtastic service page
          </RouteLink>
        ) : null}
      </div>
    </div>
  );
}
