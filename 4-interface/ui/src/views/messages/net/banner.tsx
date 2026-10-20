// The banner above a network's conversations (os/6-transport/README.md §4, §10): the adapter's honest copy (what this network is and
// is not), the dropped-text counter ("N texts dropped while 111 was busy"), and, when the network needs something, the one line that
// says what. The copy comes from the API's `Network.copy` (the adapter manifest), so this file holds no per-network words.
import { hrefTo } from "../../../app/location";
import { ButtonLink } from "../../../components/button";
import { StatusDot, type DotState } from "../../../components/status-dot";
import { useStatus } from "../../../lib/api/hooks";
import type { Network } from "../../../lib/api/types";
import { buildPrompt } from "../../../lib/prompts";
import { AgentButton, useAgentReachable } from "../../devices/agent-button";
import { NetChip } from "./glyph";
import { droppedText, networkStateLine, networkStateWord } from "./model";

const DOT: Record<string, DotState> = { live: "live", not_set: "unset", error: "error", disabled: "unknown" };

export function NetworkBanner({ network }: { network: Network }) {
  const status = useStatus();
  const reachable = useAgentReachable();
  const line = networkStateLine(network);
  const dropped = droppedText(network.dropped);
  const needs = network.state !== "live";
  return (
    <div className="flex flex-col gap-3 border border-border p-4" data-testid="network-banner">
      <div className="flex flex-wrap items-center gap-3">
        <NetChip chip={network.chip} />
        <span className="font-mono text-sm text-foreground">{network.label}</span>
        <StatusDot state={Object.prototype.hasOwnProperty.call(DOT, network.state) ? DOT[network.state]! : "unknown"} label={false} name={network.label} />
        <span className="font-mono text-[10px] uppercase tracking-widest text-muted-foreground" data-testid="network-state">
          {networkStateWord(network.state)}
        </span>
      </div>
      <p className="max-w-prose font-mono text-sm leading-relaxed text-foreground" data-testid="network-copy">
        {network.copy.banner}
      </p>
      {network.account ? <p className="break-all font-mono text-[11px] text-muted-foreground">speaks as {network.account}</p> : null}
      {line ? (
        <p role="status" className="max-w-prose font-mono text-xs leading-relaxed text-foreground" data-testid="network-line">
          {line}
        </p>
      ) : null}
      {dropped ? (
        <p role="status" className="max-w-prose font-mono text-xs leading-relaxed text-foreground" data-testid="dropped">
          {dropped}. The oldest were dropped first, and this counter is how you know.
        </p>
      ) : null}
      {needs ? (
        <div className="flex flex-wrap items-start gap-3">
          <AgentButton
            label="Set it up with the agent"
            from="Messages"
            goal="bridge.setup"
            variant="default"
            reachable={reachable}
            prompt={() => buildPrompt("bridge.setup", { kind: "node" }, { status })}
          />
          <ButtonLink href={hrefTo("/settings/l6")} variant="default">
            Transport settings
          </ButtonLink>
        </div>
      ) : null}
    </div>
  );
}
