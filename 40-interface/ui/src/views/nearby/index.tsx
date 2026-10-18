// Nearby (os/messaging/README.md section 14 and 16, interface README 6.4): the nodes a Meshtastic board has heard, grouped by
// source ("heard via Meshtastic on <board>"). Everything a node reports is an untrusted RF-origin string: it is drawn as text
// only (cleaned and cut), never as a link, and it never goes into a prompt. A Meshtastic node is not a contact. The radar and
// map views stay Layer 7's.
import { hrefTo } from "../../app/location";
import { useNow } from "../../app/hooks";
import { Badge } from "../../components/badge";
import { ButtonLink } from "../../components/button";
import { Empty } from "../../components/empty";
import { Panel } from "../../components/panel";
import { SectionHeader } from "../../components/section-header";
import { useApiMeta, useConnection, useDevices, useNearby, useServices, useStatus } from "../../lib/api/hooks";
import type { NearbyNode, NearbySource } from "../../lib/api/types";
import { Adv } from "../../lib/mode";
import { NEARBY_QUESTION, questionPrompt } from "../../lib/prompts";
import { AgentButton, useAgentReachable } from "../devices/agent-button";
import { RouteLink } from "../services/shared";
import {
  displayName,
  heardSummary,
  heardLine,
  hopsText,
  keyText,
  nearbyState,
  plain,
  positionText,
  shortName,
  signalText,
  sortedNodes,
  sourceHeading,
  utilizationText,
} from "./model";
import { nearbySentence, radarArt } from "./radar";

const ART = radarArt();
const TEXT = "font-mono text-xs leading-relaxed text-muted-foreground";
/** Text a radio supplied: its own direction, isolated from the page around it, wrapped wherever it must be. */
const RF_TEXT = "min-w-0 break-words [unicode-bidi:isolate]";

function NodeItem({ node, now }: { node: NearbyNode; now: number }) {
  const short = shortName(node);
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
        <span className="font-mono text-[10px] uppercase tracking-widest text-muted-foreground">{heardLine(node, now)}</span>
      </div>
      {facts.length > 0 ? <p className={TEXT}>{facts.join(" · ")}</p> : null}
      {position ? <p className={TEXT}>position, as it reports it: {position}</p> : null}
      <Adv>
        <p dir="auto" className={`${RF_TEXT} font-mono text-[11px] leading-snug text-muted-foreground`}>
          id {plain(node.id, 24)} · number {Number.isFinite(node.num) ? node.num : "unknown"}
          {key ? ` · key ${key}` : ""}
        </p>
      </Adv>
    </li>
  );
}

function SourceList({ source, now }: { source: NearbySource; now: number }) {
  const devices = useDevices();
  const heading = sourceHeading(source, devices);
  const nodes = sortedNodes(source.nodes);
  return (
    <Panel title="meshtastic" suffix="nodes heard" flush>
      <div className="border-b border-border px-4 py-3">
        <h2 dir="auto" className={`${RF_TEXT} font-mono text-sm text-foreground`}>
          {heading}
        </h2>
        <p className={`mt-1 ${TEXT}`}>{nodes.length === 1 ? "1 node" : `${nodes.length} nodes`}</p>
      </div>
      {nodes.length === 0 ? (
        <p className={`px-4 py-3 ${TEXT}`}>This board has not heard another node yet.</p>
      ) : (
        <ul className="divide-y divide-border">
          {nodes.map((n) => (
            <NodeItem key={`${source.id}/${n.id}`} node={n} now={now} />
          ))}
        </ul>
      )}
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
          <p className="font-mono text-sm leading-relaxed text-foreground">{heardSummary(nearby)}</p>
          {state.text ? (
            <p role="status" className="max-w-prose border border-dashed border-border p-3 font-mono text-xs leading-relaxed text-foreground" data-testid="nearby-stale">
              {state.text}
            </p>
          ) : null}
          {nearby.map((s) => (
            <SourceList key={s.id} source={s} now={now} />
          ))}
          <p className={`max-w-prose ${TEXT}`}>
            Names, positions and times are what each node says about itself over the air. They are shown as plain text and are never links. A time comes from the node&apos;s own clock, so it is a hint, not proof that
            it is in range now. A Meshtastic node is not a contact: it is a different network with its own identities, and you cannot message it from here.
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

      <Panel title="radar" tint>
        <div className="flex flex-col items-center gap-3">
          <pre role="img" aria-label="radar placeholder" className="max-w-full overflow-hidden font-mono text-xs leading-none text-muted-foreground">
            {ART}
          </pre>
          <p className="max-w-prose text-center font-mono text-xs leading-relaxed text-muted-foreground">
            The @ in the middle is this node. Radar and map views arrive with Layer 7 (Relay &amp; presence); the list above is what the board has heard so far.
          </p>
        </div>
      </Panel>

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
