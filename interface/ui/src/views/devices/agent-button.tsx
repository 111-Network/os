// The hand-off button used across Devices and Nearby: stage the prepared prompt and open the Agent view.
// When the local agent is not running the same button reveals the prompt text instead, with a copy button.
// Nothing is ever sent from here (spec section 8).
import { useId, useState } from "react";
import { navigate } from "wouter/use-hash-location";
import { hrefTo } from "../../app/location";
import { Button, ButtonLink, type ButtonVariant } from "../../components/button";
import { CopyButton } from "../../components/copy-button";
import { useServices } from "../../lib/api/hooks";
import { agentHrefFor, stagePrompt, type PromptGoal } from "../../lib/prompts";
import { REQUEST_COPY_LABEL, REQUEST_HIDE_LABEL, REQUEST_LABEL } from "../services/model";

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
  /** Route of the KB recipe for this thing (`/kb/...`). With the agent down it is the primary action. */
  recipe?: string | null;
}

export function AgentButton({ label, prompt, from, goal, variant = "primary", reachable, className, recipe = null }: AgentButtonProps) {
  const [steps, setSteps] = useState<string | null>(null); // the prepared request, while shown
  const panelId = useId();

  const onClick = () => {
    if (reachable) {
      navigate(agentHrefFor(stagePrompt(prompt(), from, goal)));
      return;
    }
    setSteps((open) => (open === null ? prompt() : null));
  };

  const recipeFirst = !reachable && recipe !== null;
  return (
    <div className={className}>
      <div className="flex flex-wrap items-start gap-3">
        {recipeFirst ? (
          <ButtonLink variant="primary" href={hrefTo(recipe)}>
            Read the recipe
          </ButtonLink>
        ) : null}
        <Button
          variant={recipeFirst ? "default" : variant}
          onClick={onClick}
          aria-expanded={reachable ? undefined : steps !== null}
          aria-controls={reachable ? undefined : panelId}
        >
          {reachable ? label : steps !== null ? REQUEST_HIDE_LABEL : REQUEST_LABEL}
        </Button>
      </div>
      <div id={panelId} hidden={steps === null} className="mt-3 max-w-prose">
        {steps !== null ? (
          <div className="flex flex-col gap-2">
            <p className="font-mono text-xs leading-relaxed text-muted-foreground">
              The local agent is not running right now. This is the request it would get. {recipe !== null ? "The recipe has the steps. " : ""}Copy it for when the agent is back.
            </p>
            <pre
              tabIndex={0}
              className="max-h-64 overflow-auto whitespace-pre-wrap break-words border border-border bg-secondary/20 p-3 font-mono text-xs leading-relaxed text-muted-foreground focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-foreground focus-visible:outline-none"
            >
              {steps}
            </pre>
            <CopyButton text={steps} label={REQUEST_COPY_LABEL} ariaLabel="Copy the request" />
          </div>
        ) : null}
      </div>
    </div>
  );
}
