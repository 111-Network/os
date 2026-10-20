// One network (os/6-transport/README.md §10): its banner, its conversations split into channels and DMs, a way to open a DM by the
// network's own address (or from what Nearby heard), and the relays that touch it.
import { useState, type FormEvent } from "react";
import { navigate } from "wouter/use-hash-location";
import { redirectIfUnauthenticated } from "../../../app/AuthGate";
import { hrefTo } from "../../../app/location";
import { Button, ButtonLink } from "../../../components/button";
import { Empty } from "../../../components/empty";
import { Input } from "../../../components/input";
import { Panel } from "../../../components/panel";
import { SectionHeader } from "../../../components/section-header";
import { apiStore, getAdapter } from "../../../lib/api";
import { addressProblem, normalizeAddress } from "../../../lib/api/bridge";
import { useNearby, useNetwork, useNetworkConversations } from "../../../lib/api/hooks";
import { foreignPath } from "../../../lib/api/networks";
import type { Network } from "../../../lib/api/types";
import { errorSentence } from "../model";
import { NetworkBanner } from "./banner";
import { SplitConversations } from "./conversations";
import { EdgesPanel } from "./edges";
import { plain } from "./model";

/** Which Nearby source kind feeds each network's DM addresses. */
const NEARBY_KIND: Record<string, string> = { meshtastic: "meshtastic", lxmf: "reticulum" };

export function OpenDmForm({ network }: { network: Network }) {
  const syntax = network.caps.recipient_syntax;
  const [value, setValue] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const nearby = useNearby();
  const kind = NEARBY_KIND[network.id];
  const heard = (nearby ?? []).filter((s) => s.kind === kind).flatMap((s) => s.nodes);
  if (!syntax || !network.caps.kinds.includes("dm") || network.state === "disabled") return null;

  const open = async (native: string) => {
    setBusy(true);
    setError(null);
    try {
      const adapter = await getAdapter();
      const c = await adapter.networks.openDm(network.id, native);
      void apiStore.refreshConversations(network.id);
      navigate(foreignPath(network.id, c.id));
    } catch (err) {
      if (redirectIfUnauthenticated(err)) return;
      setError(errorSentence(err, "Could not open that conversation."));
    } finally {
      setBusy(false);
    }
  };
  const submit = (e: FormEvent) => {
    e.preventDefault();
    const native = normalizeAddress(value);
    const problem = addressProblem(syntax, native);
    if (problem) return setError(problem);
    void open(native);
  };
  return (
    <Panel title="direct message" suffix={network.label}>
      <form className="flex flex-col gap-3" onSubmit={submit} noValidate>
        <label className="flex flex-col gap-1">
          <span className="font-mono text-[10px] uppercase tracking-widest text-foreground">{syntax.label}</span>
          <Input value={value} autoComplete="off" spellCheck={false} placeholder={syntax.example} onChange={(e) => {
              setValue(e.target.value);
              setError(null);
            }} />
        </label>
        <p className="max-w-prose font-mono text-xs leading-relaxed text-muted-foreground">
          {network.copy.dm_note ?? "A foreign address is never a contact."} It opens a conversation here; nothing is sent until you write.
        </p>
        <div>
          <Button type="submit" size="sm" loading={busy}>
            open a DM
          </Button>
        </div>
        {error ? (
          <p role="alert" className="font-mono text-xs text-destructive">
            {error}
          </p>
        ) : null}
      </form>
      {heard.length > 0 ? (
        <div className="mt-4 flex flex-col gap-2" data-testid="heard-picks">
          <p className="font-mono text-[10px] uppercase tracking-widest text-muted-foreground">or pick one this node heard</p>
          <ul className="flex flex-col gap-1">
            {heard.slice(0, 12).map((n) => (
              <li key={n.id} className="flex flex-wrap items-center justify-between gap-2 font-mono text-xs">
                <span dir="auto" className="min-w-0 break-words text-foreground [unicode-bidi:isolate]">
                  {plain(n.long_name || n.short_name || n.id, 40)}
                </span>
                <Button size="sm" disabled={busy || addressProblem(syntax, n.id) !== null} onClick={() => void open(n.id)} aria-label={`Open a DM with ${plain(n.long_name || n.id, 40)}`}>
                  DM
                </Button>
              </li>
            ))}
          </ul>
        </div>
      ) : null}
    </Panel>
  );
}

export default function NetworkPage({ network: id }: { network: string }) {
  const network = useNetwork(id);
  const convs = useNetworkConversations(id);
  const back = (
    <ButtonLink href={hrefTo("/messages")} size="sm">
      All messages
    </ButtonLink>
  );
  if (!network) {
    return (
      <div className="flex flex-col gap-6">
        <SectionHeader kicker="network" title="Network" level={1} end={back} />
        <Empty kicker="not found" title="No such network">
          This node does not list a network with that name.
        </Empty>
      </div>
    );
  }
  return (
    <div className="flex flex-col gap-6">
      <SectionHeader kicker="network" title={network.label} level={1} end={back} />
      <NetworkBanner network={network} />
      {network.state === "disabled" ? (
        <Empty kicker="off" title={`${network.label} is off`}>
          Nothing of it runs while it is off. Turn it on in Settings, Transport, where its size and licence are shown first.
        </Empty>
      ) : convs === undefined ? (
        <p role="status" className="font-mono text-xs text-muted-foreground">
          Loading conversations
        </p>
      ) : convs.length === 0 ? (
        <Empty title="No conversations yet">Channels appear here as the board reports them; a DM opens from an address or from Nearby.</Empty>
      ) : (
        <SplitConversations convs={convs} idPrefix="net" />
      )}
      <OpenDmForm network={network} />
      <div className="flex flex-wrap gap-3">
        <ButtonLink href={hrefTo("/messages/send")} size="sm">
          Send to networks…
        </ButtonLink>
      </div>
      <EdgesPanel network={network.id} />
    </div>
  );
}
