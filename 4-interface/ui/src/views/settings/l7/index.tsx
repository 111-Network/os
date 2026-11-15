// Settings, Relay & presence: /settings/l7 (os/7-relay/README.md §12). Presence (the heartbeat per transport, shown and inert, the airtime
// line, the bans), the gateway and its carrier, paired hubs, discovery, updates and metered networks. Every change is a call the node
// answers, and the page shows the answer; the node's own consent texts are shown verbatim before anything they cover turns on. This page
// is its own lazy chunk (at most 64 KB gzip, D21): nothing of it is loaded until the person opens it.
import { Empty } from "../../../components/empty";
import { SectionHeader } from "../../../components/section-header";
import { StatusDot } from "../../../components/status-dot";
import { useConnection, useFabric, useServices } from "../../../lib/api/hooks";
import { layerBySlug } from "../../../lib/layers";
import { dotState } from "../../../lib/nav";
import { reasonText } from "../../services/model";
import { FEEDBACK } from "../row";
import { useRelayStatus } from "./actions";
import { DiscoverySection } from "./discovery";
import { GatewaySection } from "./gateway";
import { MeteredSection } from "./metered";
import { PairingSection } from "./pairing";
import { PresenceSection } from "./presence";
import { UpdatesSection } from "./updates";

export function RelaySettings() {
  const layer = layerBySlug("l7")!;
  const relay = useRelayStatus();
  const fabric = useFabric();
  const services = useServices();
  const { disconnected } = useConnection();
  const service = services?.find((s) => s.id === "gateway") ?? null;
  // Only a node that says it is not a hub is gated; a node that has not reported yet is left to answer for itself.
  const hubGate = fabric !== null && fabric.settings.hub_role === false;

  return (
    <div className="flex flex-col gap-6" data-testid="relay-settings">
      <SectionHeader level={1} kicker={`Layer ${layer.n}`} title={layer.title} />

      <div className="flex flex-col gap-2">
        <p className={`max-w-prose text-muted-foreground ${FEEDBACK}`}>{layer.description}</p>
        {service ? (
          <div className="flex flex-wrap items-center gap-3">
            <StatusDot state={dotState(service.state, disconnected)} />
            <span className="font-mono text-sm text-foreground">{reasonText(service)}</span>
          </div>
        ) : null}
        <p className={`max-w-prose text-muted-foreground ${FEEDBACK}`}>
          The agent can propose turning a gateway on, opening the pairing screen and sending now. It never accepts a confirmation, never types a code and never sees a secret.
        </p>
      </div>

      {relay === null ? (
        <Empty kicker="unavailable" title="Relay and presence are not reported yet">
          The node has not sent its Layer 7 state. If this node is older than Layer 7 it has none; otherwise this fills in as soon as it answers.
        </Empty>
      ) : (
        <>
          <PresenceSection relay={relay} />
          <GatewaySection relay={relay} hubGate={hubGate} />
          <PairingSection relay={relay} hubGate={hubGate} />
          <DiscoverySection relay={relay} hubGate={hubGate} />
          <UpdatesSection relay={relay} hubGate={hubGate} />
          <MeteredSection relay={relay} />
        </>
      )}
    </div>
  );
}

export default RelaySettings;
