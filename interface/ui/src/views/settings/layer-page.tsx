// Settings > one layer page (/settings/l0 .. /settings/l9). Structure only this cycle: description,
// the status fields the layer owns today, "configurable here (coming)", and a hand-off to the agent.
import { useLocation } from "wouter";
import { Badge } from "../../components/badge";
import { Button } from "../../components/button";
import { Panel } from "../../components/panel";
import { SectionHeader } from "../../components/section-header";
import { SpecTable } from "../../components/spec-table";
import { useProposals, useServices, useStatus } from "../../lib/api/hooks";
import type { LayerInfo } from "../../lib/layers";
import { agentHrefFor, stagePrompt } from "../../lib/prompts";
import { toSpecRows } from "./facts";
import { layerAskPrompt, layerStatusRows } from "./model";
import { FEEDBACK } from "./row";

const H2 = "font-mono text-[10px] uppercase tracking-widest text-foreground";

export function LayerPage({ layer }: { layer: LayerInfo }) {
  const status = useStatus();
  const services = useServices();
  const proposals = useProposals();
  const [, navigate] = useLocation();

  const openProposals = proposals ? proposals.filter((p) => p.decision === null).length : null;
  const rows = layerStatusRows(layer.n, status, { services, openProposals });

  const ask = () => {
    const { text, from, goal } = layerAskPrompt(layer.n, layer.title, status);
    navigate(agentHrefFor(stagePrompt(text, from, goal)));
  };

  return (
    <div className="flex flex-col gap-6">
      <SectionHeader level={1} kicker={`Layer ${layer.n}`} title={layer.title} />

      <Panel title={`l${layer.n}`} suffix="about">
        <h2 className={H2}>What it is</h2>
        <p className={`mt-2 max-w-prose text-muted-foreground ${FEEDBACK}`}>{layer.description}</p>
      </Panel>

      <Panel title="status" suffix={`l${layer.n}`} flush>
        <div className="border-b border-border px-4 py-3">
          <h2 className={H2}>Status this layer owns today</h2>
        </div>
        <SpecTable bare rows={toSpecRows(rows)} />
        <div className="flex flex-wrap items-center justify-between gap-3 border-t border-border px-4 py-3 font-mono text-xs">
          <span className="text-muted-foreground">Settings for this layer</span>
          <Badge>configurable here (coming)</Badge>
        </div>
      </Panel>

      <Panel title="agent" suffix={`l${layer.n}`}>
        <h2 className={H2}>Ask the agent</h2>
        <p className={`mt-2 max-w-prose text-muted-foreground ${FEEDBACK}`}>
          Opens the Agent with a prepared question about this layer. You can edit it first, and nothing is sent until you send it.
        </p>
        <div className="mt-4">
          <Button onClick={ask}>Ask the agent about {layer.title}</Button>
        </div>
      </Panel>
    </div>
  );
}
