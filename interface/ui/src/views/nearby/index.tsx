// Nearby (spec 6.4, stub until Layer 7): other people's nodes around you. v1 says what 111d knows in one
// sentence and reserves the canvas for the radar and map. No fake peers.
import { Panel } from "../../components/panel";
import { SectionHeader } from "../../components/section-header";
import { useStatus } from "../../lib/api/hooks";
import { AgentButton, useAgentReachable } from "../devices/agent-button";
import { nearbySentence, radarArt } from "./radar";

const ART = radarArt();

export default function NearbyView() {
  const status = useStatus();
  const reachable = useAgentReachable();
  const sentence = nearbySentence(status?.transport_state);

  return (
    <div className="flex flex-col gap-6">
      <SectionHeader kicker="nearby" title="Nearby" level={1} />
      <p className="max-w-prose font-mono text-sm leading-relaxed text-foreground">{sentence}</p>

      <Panel title="radar" tint>
        <div className="flex flex-col items-center gap-3">
          <pre
            role="img"
            aria-label="radar placeholder"
            className="max-w-full overflow-hidden font-mono text-xs leading-none text-muted-foreground"
          >
            {ART}
          </pre>
          <p className="max-w-prose text-center font-mono text-xs leading-relaxed text-muted-foreground">
            The @ in the middle is this node. Radar and map views arrive with Layer 7 (Relay &amp; presence).
          </p>
        </div>
      </Panel>

      <div>
        <AgentButton
          label="Ask the agent who is nearby"
          from="Nearby"
          goal="check"
          reachable={reachable}
          prompt={() =>
            `Who is nearby? Tell me what this node can hear on the mesh right now, and what I would need to see more.\n\nContext from the dashboard: transport state is "${status?.transport_state ?? "unknown"}".`
          }
        />
      </div>
    </div>
  );
}
