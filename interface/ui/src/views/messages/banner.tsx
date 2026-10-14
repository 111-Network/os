// The one line above the Messages list when the `fabric` ("Messaging") service is not LIVE, or the store is full
// (os/messaging/README.md section 15, interface README 7.2). The sentence is the node's own; the agent button is the fix.
// What the button asks follows the node's `reason_code` (model.bannerAgent), not just its state: an unreachable home hub
// is a fix, and a hub with no members is offered "let a node use this hub", whatever state the row reports.
import { useServices, useStatus, useFabric } from "../../lib/api/hooks";
import { questionPrompt } from "../../lib/prompts";
import { AgentButton, useAgentReachable } from "../devices/agent-button";
import { bannerAgent, messagingBanner } from "./model";

export function MessagingBanner() {
  const services = useServices();
  const fabric = useFabric();
  const status = useStatus();
  const reachable = useAgentReachable();
  const service = services?.find((s) => s.id === "fabric") ?? null;
  const banner = messagingBanner(service, fabric);
  if (!banner) return null;
  const agent = bannerAgent(banner, service?.reason_code);
  return (
    <div className="flex flex-col gap-3 border border-border p-4" data-testid="messaging-banner">
      <p className="font-mono text-[10px] uppercase tracking-widest text-muted-foreground">{banner.kind === "error" ? "messaging problem" : banner.kind === "starting" ? "messaging starting" : "messaging not set up"}</p>
      <p className="max-w-prose font-mono text-sm leading-relaxed text-foreground">{banner.text}</p>
      {agent ? (
        <AgentButton
          label={agent.label}
          from="Messages"
          goal={agent.goal}
          reachable={reachable}
          variant={agent.primary ? "primary" : "default"}
          prompt={() => questionPrompt(agent.question, service ? { kind: "service", service } : { kind: "node" }, { status })}
        />
      ) : null}
    </div>
  );
}
