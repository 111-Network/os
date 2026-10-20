// Settings, Transport: /settings/l6 (os/60-transport/README.md §10 "UI", §11). Adapters and their order, the radio region and callsign (a
// human fact; "not set" until then), the rnsd posture and the phone-relay join, relays at a glance, recipient lists, optional components
// with their sizes and licence notice, and the confirmations the node asks for (licensed boards, uplinked channels). Every change is a call
// the node answers, and the page shows the answer.
import { hrefTo } from "../../../app/location";
import { Badge } from "../../../components/badge";
import { ButtonLink } from "../../../components/button";
import { Empty } from "../../../components/empty";
import { Panel } from "../../../components/panel";
import { SectionHeader } from "../../../components/section-header";
import { StatusDot } from "../../../components/status-dot";
import { useBridgeSettings, useComponents, useConnection, useEdges, useNetworks, useServices, useStatus, useTransport } from "../../../lib/api/hooks";
import { layerBySlug } from "../../../lib/layers";
import { dotState } from "../../../lib/nav";
import { buildPrompt } from "../../../lib/prompts";
import { AgentButton, useAgentReachable } from "../../devices/agent-button";
import { reasonText } from "../../services/model";
import { FEEDBACK, Row } from "../row";
import { AdapterRows, OrderRow } from "./adapters";
import { ComponentsRow } from "./components";
import { GatesRows } from "./gates";
import { ListsRow } from "./lists";
import { JoinRow, PostureRows, RegionRows } from "./radio";

const H2 = "font-mono text-[10px] uppercase tracking-widest text-foreground";

function RelaysRow() {
  const edges = useEdges();
  const mine = edges ?? [];
  const paused = mine.filter((e) => e.paused).length;
  const tooLong = mine.reduce((n, e) => n + e.counters.refused, 0);
  const shed = mine.reduce((n, e) => n + e.counters.shed, 0);
  return (
    <Row id="relays" label="Relays" help="A relay copies one channel into another, one way. You start, pause and remove them on the network's page in Messages, where the exact warning is shown.">
      <div className="flex flex-col gap-3">
        {edges === null ? (
          <p className={`text-muted-foreground ${FEEDBACK}`}>Not reported yet.</p>
        ) : mine.length === 0 ? (
          <p className={`text-muted-foreground ${FEEDBACK}`}>No relays.</p>
        ) : (
          <p className={`text-foreground ${FEEDBACK}`} data-testid="relays-summary">
            {mine.length === 1 ? "1 relay" : `${mine.length} relays`}
            {paused > 0 ? `, ${paused} paused` : ""}
            {shed > 0 ? `, ${shed} ${shed === 1 ? "copy" : "copies"} shed` : ""}
            {tooLong > 0 ? `, ${tooLong} too long to relay` : ""}.
          </p>
        )}
        <div>
          <ButtonLink href={hrefTo("/messages/net/meshtastic")} size="sm">
            Manage relays in Messages
          </ButtonLink>
        </div>
      </div>
    </Row>
  );
}

export function TransportSettings() {
  const layer = layerBySlug("l6")!;
  const networks = useNetworks();
  const settings = useBridgeSettings();
  const components = useComponents();
  const transport = useTransport();
  const status = useStatus();
  const services = useServices();
  const { disconnected } = useConnection();
  const reachable = useAgentReachable();
  const service = services?.find((s) => s.id === "transport") ?? null;
  const list = networks ?? [];

  return (
    <div className="flex flex-col gap-6" data-testid="transport-settings">
      <SectionHeader level={1} kicker={`Layer ${layer.n}`} title={layer.title} />

      <div className="flex flex-col gap-2">
        <p className={`max-w-prose text-muted-foreground ${FEEDBACK}`}>{layer.description}</p>
        {service ? (
          <div className="flex flex-wrap items-center gap-3">
            <StatusDot state={dotState(service.state, disconnected)} />
            <span className="font-mono text-sm text-foreground">{reasonText(service)}</span>
          </div>
        ) : null}
      </div>

      {networks === null && transport === null ? (
        <Empty kicker="unavailable" title="Transport is not reported yet">
          The node has not sent its network state. If this node is older than Layer 6 it has none; otherwise this fills in as soon as it answers.
        </Empty>
      ) : (
        <>
          <Panel title="networks" suffix="on and off" flush>
            <AdapterRows networks={list} />
            <OrderRow settings={settings} networks={list} />
          </Panel>

          {transport ? (
            <Panel title="radio" suffix="region" flush>
              <RegionRows t={transport} />
            </Panel>
          ) : null}

          {transport ? (
            <Panel title="reticulum" suffix="posture" flush>
              <PostureRows t={transport} />
              <JoinRow t={transport} />
            </Panel>
          ) : null}

          <Panel title="relays" suffix="and lists" flush>
            <RelaysRow />
            <ListsRow settings={settings} networks={list} />
          </Panel>

          <Panel title="components" suffix="optional" flush>
            <ComponentsRow components={components} networks={list} />
          </Panel>

          <Panel title="confirmations" suffix="live" flush>
            <GatesRows settings={settings} />
          </Panel>
        </>
      )}

      <Panel title="agent" suffix="l6">
        <h2 className={H2}>Ask the agent</h2>
        <p className={`mt-2 max-w-prose text-muted-foreground ${FEEDBACK}`}>
          Opens the Agent with a prepared message. You can edit it first, and nothing is sent until you send it. The agent can only propose: every network, relay and confirmation is yours to turn on.
        </p>
        <div className="mt-4 flex flex-wrap items-start gap-3">
          <AgentButton label="Set up a network or a relay with the agent" from="Transport settings" goal="bridge.setup" variant="primary" reachable={reachable} prompt={() => buildPrompt("bridge.setup", { kind: "node" }, { status })} />
          <AgentButton
            label="Ask the agent to fix the transport"
            from="Transport settings"
            goal="transport.fix"
            variant="default"
            reachable={reachable}
            prompt={() => buildPrompt("transport.fix", service ? { kind: "service", service } : { kind: "node" }, { status })}
          />
        </div>
        <Badge className="mt-4 inline-block">{layer.title}</Badge>
      </Panel>
    </div>
  );
}

export default TransportSettings;
