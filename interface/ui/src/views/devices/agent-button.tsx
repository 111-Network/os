// The hand-off button used across Devices and Nearby: stage the prepared prompt and open the Agent view.
// When the local agent is not running the same button reveals the prompt text instead, with a copy button.
// Nothing is ever sent from here (spec section 8).
import { useId, useState } from "react";
import { navigate } from "wouter/use-hash-location";
import { Button, type ButtonVariant } from "../../components/button";
import { CopyButton } from "../../components/copy-button";
import { useServices } from "../../lib/api/hooks";
import { agentHrefFor, stagePrompt, type PromptGoal } from "../../lib/prompts";

/** True while the agent service is live, and also before the first services snapshot (the Agent view explains a down agent). */
export function useAgentReachable(): boolean {
  const services = useServices();
  if (services === null) return true;
  return services.find((s) => s.id === "agent")?.state === "live";
}

export interface AgentButtonProps {
  /** Button text while the agent is reachable. */
  label: string;
  /** Called on click, so the prompt reflects the registry at that moment. */
  prompt: () => string;
  /** Short origin label for the Agent view banner. */
  from: string;
  goal: PromptGoal;
  variant?: ButtonVariant;
  reachable: boolean;
  className?: string;
}

export function AgentButton({ label, prompt, from, goal, variant = "primary", reachable, className }: AgentButtonProps) {
  const [steps, setSteps] = useState<string | null>(null);
  const panelId = useId();

  const onClick = () => {
    if (reachable) {
      navigate(agentHrefFor(stagePrompt(prompt(), from, goal)));
      return;
    }
    setSteps((open) => (open === null ? prompt() : null));
  };

  return (
    <div className={className}>
      <Button
        variant={variant}
        onClick={onClick}
        aria-expanded={reachable ? undefined : steps !== null}
        aria-controls={reachable ? undefined : panelId}
      >
        {reachable ? label : steps !== null ? "Hide steps" : "Show steps"}
      </Button>
      <div id={panelId} hidden={steps === null} className="mt-3 max-w-prose">
        {steps !== null ? (
          <div className="flex flex-col gap-2">
            <p className="font-mono text-xs leading-relaxed text-muted-foreground">
              The local agent is not running right now. This is the prompt it would get. Copy it for when the agent is back.
            </p>
            <pre
              tabIndex={0}
              className="max-h-64 overflow-auto whitespace-pre-wrap break-words border border-border bg-secondary/20 p-3 font-mono text-xs leading-relaxed text-muted-foreground focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-foreground focus-visible:outline-none"
            >
              {steps}
            </pre>
            <CopyButton text={steps} label="copy prompt" ariaLabel="Copy the prompt" />
          </div>
        ) : null}
      </div>
    </div>
  );
}
