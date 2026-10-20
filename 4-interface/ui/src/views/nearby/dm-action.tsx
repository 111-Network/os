// "Send a Meshtastic DM from this node (readable, not 111-sealed)" (os/6-transport/README.md §5, §10): opens a foreign DM with a node Nearby
// heard. The label is the network's own (`Network.copy.dm_note`, from its manifest). A heard node is never a contact: this opens a
// conversation in Messages under that network, where the text is labelled for what it is, and nothing is sent until the person writes.
import { useState } from "react";
import { navigate } from "wouter/use-hash-location";
import { redirectIfUnauthenticated } from "../../app/AuthGate";
import { Button } from "../../components/button";
import { apiStore, getAdapter, isApiError } from "../../lib/api";
import { addressProblem } from "../../lib/api/bridge";
import { useNetworks } from "../../lib/api/hooks";
import { foreignPath } from "../../lib/api/networks";
import type { NearbyNode } from "../../lib/api/types";

/** The network a presence source feeds: Meshtastic nodes are Meshtastic DMs, Reticulum announces are LXMF ones. */
const NETWORK_OF_SOURCE: Record<string, string> = { meshtastic: "meshtastic", reticulum: "lxmf" };

export function DmAction({ node, sourceKind }: { node: Pick<NearbyNode, "id" | "long_name">; sourceKind: string }) {
  const networks = useNetworks();
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const id = Object.prototype.hasOwnProperty.call(NETWORK_OF_SOURCE, sourceKind) ? NETWORK_OF_SOURCE[sourceKind]! : null;
  const network = id ? networks?.find((n) => n.id === id) : undefined;
  if (!network || network.state === "disabled" || !network.caps.kinds.includes("dm")) return null;
  // The id must be an address this network can send to; a hostile or malformed id offers nothing.
  if (addressProblem(network.caps.recipient_syntax, node.id) !== null) return null;
  const label = network.copy.dm_note?.trim() || `Send a ${network.label} DM from this node (readable, not 111-sealed)`;

  const open = async () => {
    setBusy(true);
    setError(null);
    try {
      const adapter = await getAdapter();
      const c = await adapter.networks.openDm(network.id, node.id);
      void apiStore.refreshConversations(network.id);
      navigate(foreignPath(network.id, c.id));
    } catch (err) {
      if (redirectIfUnauthenticated(err)) return;
      setError(isApiError(err) ? err.problem.detail || err.problem.title : "Could not open that conversation.");
    } finally {
      setBusy(false);
    }
  };
  return (
    <div className="mt-1 flex flex-col gap-1">
      <div>
        <Button size="sm" loading={busy} onClick={() => void open()} data-testid="nearby-dm">
          {label}
        </Button>
      </div>
      {error ? (
        <p role="alert" className="font-mono text-xs text-destructive">
          {error}
        </p>
      ) : null}
    </div>
  );
}
