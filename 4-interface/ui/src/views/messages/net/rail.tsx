// The Messages rail (os/60-transport/README.md §10): "111" first, then one entry per enabled network in the user's preferred order,
// each split into channels and DMs. The "111" entry is the list this rail sits on. A foreign DM raises the unread number here and
// never the header badge. Disabled networks are not listed (they cost nothing and show in Settings).
import { hrefTo } from "../../../app/location";
import { Badge } from "../../../components/badge";
import { ButtonLink } from "../../../components/button";
import { foreignUnread } from "../../../lib/attention";
import { useNetworkConversations, useNetworks } from "../../../lib/api/hooks";
import { enabledNetworks, foreignPath } from "../../../lib/api/networks";
import type { Network } from "../../../lib/api/types";
import { cx, FOCUS_RING } from "../../../lib/cx";
import { SplitConversations } from "./conversations";
import { NetChip } from "./glyph";
import { networkStateWord, splitSummary } from "./model";

function RailEntry({ network }: { network: Network }) {
  const convs = useNetworkConversations(network.id);
  const unread = foreignUnread(convs);
  return (
    <li className="flex flex-col gap-2 border-b border-border p-4 last:border-b-0" data-testid={`rail-${network.id}`}>
      <a href={hrefTo(foreignPath(network.id))} className={cx("flex min-h-11 flex-wrap items-center gap-x-3 gap-y-1 font-mono", FOCUS_RING)}>
        <NetChip chip={network.chip} />
        <span className="text-sm text-foreground">{network.label}</span>
        <span className="text-[10px] uppercase tracking-widest text-muted-foreground">{networkStateWord(network.state)}</span>
        {unread > 0 ? <Badge className="border-foreground text-foreground">{unread} unread</Badge> : null}
      </a>
      <p className="font-mono text-xs text-muted-foreground">{convs ? splitSummary(convs) : `${network.counts.conversations} conversations`}</p>
      {convs && convs.length > 0 ? <SplitConversations convs={convs} idPrefix={`rail-${network.id.replace(/[^a-z0-9]/g, "-")}`} /> : null}
    </li>
  );
}

export function NetworkRail() {
  const networks = useNetworks();
  const on = enabledNetworks(networks);
  const off = (networks ?? []).length - on.length;
  if (networks === null || networks.length === 0) return null;
  return (
    <section aria-labelledby="rail-h" className="flex flex-col gap-2" data-testid="network-rail">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <h2 id="rail-h" className="font-mono text-[10px] uppercase tracking-widest text-muted-foreground">
          other networks
        </h2>
        {on.length > 0 ? (
          <ButtonLink href={hrefTo("/messages/send")} size="sm">
            Send to networks…
          </ButtonLink>
        ) : null}
      </div>
      <ul className="border border-border">
        <li className="flex min-h-11 items-center gap-3 border-b border-border px-4 py-2 font-mono" aria-current="page" data-testid="rail-111">
          <NetChip chip="111" />
          <span className="text-sm text-foreground">111 messages</span>
          <span className="text-xs text-muted-foreground">sealed, end to end: the list below</span>
        </li>
        {on.map((n) => (
          <RailEntry key={n.id} network={n} />
        ))}
      </ul>
      {off > 0 ? (
        <p className="font-mono text-xs text-muted-foreground">
          {off === 1 ? "1 network is off and costs nothing while off." : `${off} networks are off and cost nothing while off.`}{" "}
          <a href={hrefTo("/settings/l6")} className="underline underline-offset-4">
            Turn on in Settings
          </a>
        </p>
      ) : null}
    </section>
  );
}
