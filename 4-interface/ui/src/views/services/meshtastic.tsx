// What the `meshtastic` service page adds to the generic control panel (os/5-messaging/README.md sections 14 and 16): the nodes
// heard (the list itself is Nearby), and the honest copy: what the panel transmits, whose keys protect it, which outside hosts a
// browser may contact, and the phone residual. The nodes' names are never repeated here; only how many were heard.
import { hrefTo } from "../../app/location";
import { ButtonLink } from "../../components/button";
import { Panel } from "../../components/panel";
import { useNearby } from "../../lib/api/hooks";
import { nodesHeard } from "../../lib/api/nearby";
import type { HostedPanel } from "../../lib/api/types";
import { PanelNotes } from "./panel";

const TEXT = "max-w-prose font-mono text-xs leading-relaxed text-muted-foreground";

/** "No nodes heard yet", "1 node heard", "12 nodes heard". Null before the node has answered. */
export function heardText(count: number | null): string {
  if (count === null) return "The node has not reported what it hears yet.";
  if (count === 0) return "No other Meshtastic nodes heard yet.";
  return count === 1 ? "1 Meshtastic node heard." : `${count} Meshtastic nodes heard.`;
}

export function MeshtasticDetails({ panel }: { panel: HostedPanel | null }) {
  const nearby = useNearby();
  const count = nearby === null ? null : nodesHeard(nearby);
  return (
    <div className="flex flex-col gap-6" data-testid="meshtastic-details">
      <Panel title="nodes heard" suffix="nearby" bodyClassName="space-y-3">
        <h2 className="sr-only">Nodes heard</h2>
        <p className="font-mono text-sm leading-relaxed text-foreground">{heardText(count)}</p>
        <p className={TEXT}>Meshtastic nodes are not contacts: they are a different network with its own identities, and you cannot message them from here.</p>
        <div>
          <ButtonLink size="sm" href={hrefTo("/nearby")}>
            See who is nearby
          </ButtonLink>
        </div>
      </Panel>
      <section className="flex flex-col gap-2" aria-labelledby="meshtastic-copy-h">
        <h2 id="meshtastic-copy-h" className="font-mono text-[10px] uppercase tracking-widest text-muted-foreground">
          what to know
        </h2>
        <PanelNotes id="meshtastic" optionalHosts={panel?.optional_hosts ?? []} parts={["transmits", "keys", "hosts", "residual"]} />
      </section>
    </div>
  );
}
