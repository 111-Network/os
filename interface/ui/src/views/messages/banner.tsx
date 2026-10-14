// The one line above the Messages list when the `fabric` ("Messaging") service is not LIVE, or the store is full
// (os/messaging/README.md section 15, interface README 7.2). The sentence is the node's own; the agent button is the fix.
import { useServices, useStatus, useFabric } from "../../lib/api/hooks";
import { questionPrompt } from "../../lib/prompts";
import { AgentButton, useAgentReachable } from "../devices/agent-button";
import { messagingBanner } from "./model";

const SETUP_QUESTION = "Messaging on this node is not working yet. Check why, and walk me through setting it up: enrolling with a home hub, or connecting another 111 node.";
const FIX_QUESTION = "Messaging on this node has a problem. Check what is wrong (an unreachable home hub, or a full message store) and tell me the steps to fix it.";

export function MessagingBanner() {
  const services = useServices();
  const fabric = useFabric();
  const status = useStatus();
  const reachable = useAgentReachable();
  const service = services?.find((s) => s.id === "fabric") ?? null;
  const banner = messagingBanner(service, fabric);
  if (!banner) return null;
  const setup = banner.kind === "unset";
  return (
    <div className="flex flex-col gap-3 border border-border p-4" data-testid="messaging-banner">
      <p className="font-mono text-[10px] uppercase tracking-widest text-muted-foreground">{banner.kind === "error" ? "messaging problem" : banner.kind === "starting" ? "messaging starting" : "messaging not set up"}</p>
      <p className="max-w-prose font-mono text-sm leading-relaxed text-foreground">{banner.text}</p>
      {banner.fixable ? (
        <AgentButton
          label={setup ? "Set up messaging with the agent" : "Ask the agent to fix it"}
          from="Messages"
          goal={setup ? "setup" : "fix"}
          reachable={reachable}
          variant={setup ? "primary" : "default"}
          prompt={() => questionPrompt(setup ? SETUP_QUESTION : FIX_QUESTION, service ? { kind: "service", service } : { kind: "node" }, { status })}
        />
      ) : null}
    </div>
  );
}
