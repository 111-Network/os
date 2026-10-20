// The one line above the Messages list when the `fabric` ("Messaging") service is not LIVE, or the store is full
// (os/50-messaging/README.md section 15, interface README 7.2). The sentence is the node's own; the agent button is the fix.
// What the button asks follows the node's `reason_code` (model.bannerAgent), not just its state: an unreachable home hub
// is a fix, and a hub with no members is offered "let a node use this hub", whatever state the row reports. The prepared
// message is the Layer 5 context goal (`messaging.setup` / `messaging.fix`, lib/prompts.ts) over the `fabric` service's facts.
import { hrefTo } from "../../app/location";
import { ButtonLink } from "../../components/button";
import { useServices, useStatus, useFabric } from "../../lib/api/hooks";
import { buildPrompt, type PromptGoal } from "../../lib/prompts";
import { AgentButton, useAgentReachable } from "../devices/agent-button";
import { bannerAgent, messagingBanner } from "./model";

/** The banner's choice of `fix` or `setup` as the Layer 5 context goal. */
export const messagingGoal = (goal: PromptGoal): PromptGoal => (goal === "fix" ? "messaging.fix" : "messaging.setup");

export function MessagingBanner() {
  const services = useServices();
  const fabric = useFabric();
  const status = useStatus();
  const reachable = useAgentReachable();
  const service = services?.find((s) => s.id === "fabric") ?? null;
  const banner = messagingBanner(service, fabric);
  if (!banner) return null;
  const agent = bannerAgent(banner, service?.reason_code);
  const goal = agent ? messagingGoal(agent.goal) : null;
  return (
    <div className="flex flex-col gap-3 border border-border p-4" data-testid="messaging-banner">
      <p className="font-mono text-[10px] uppercase tracking-widest text-muted-foreground">{banner.kind === "error" ? "messaging problem" : banner.kind === "starting" ? "messaging starting" : "messaging not set up"}</p>
      <p className="max-w-prose font-mono text-sm leading-relaxed text-foreground">{banner.text}</p>
      {agent && goal ? (
        <div className="flex flex-wrap items-start gap-3">
          <AgentButton
            label={agent.label}
            from="Messages"
            goal={goal}
            reachable={reachable}
            variant={agent.primary ? "primary" : "default"}
            prompt={() => buildPrompt(goal, service ? { kind: "service", service } : { kind: "node" }, { status })}
          />
          <ButtonLink href={hrefTo("/settings/l5")} variant="default">
            Messaging settings
          </ButtonLink>
        </div>
      ) : null}
    </div>
  );
}
