// Relay edges (os/60-transport/README.md §8): explicit, directed, non-transitive copies from one channel or group to another. Only a
// human starts one, and starting (or resuming a consent-paused one) goes through the consent round-trip: the node states the exact
// warning with its live add-ons, and this page echoes its hash. History is never relayed. Counters are the node's own.
import { useState, type FormEvent } from "react";
import { redirectIfUnauthenticated } from "../../../app/AuthGate";
import { announce } from "../../../app/LiveRegion";
import { Button } from "../../../components/button";
import { FIELD_CLASS } from "../../../components/input";
import { Panel } from "../../../components/panel";
import { Segmented } from "../../../components/segmented";
import { Toggle } from "../../../components/toggle";
import { apiStore, getAdapter } from "../../../lib/api";
import { edgeCountersText, edgeDestinationOnly, edgeEndpoint, edgeIsConsentPaused, edgeStateText } from "../../../lib/api/bridge";
import { useConversationsOf, useEdges, useNetworks } from "../../../lib/api/hooks";
import { enabledNetworks } from "../../../lib/api/networks";
import type { BridgeEdge, ForeignConversation, Network } from "../../../lib/api/types";
import { errorSentence } from "../model";
import { useConsentFlow } from "./consent";
import { NetChip } from "./glyph";
import { convTitle } from "./model";

const TEXT = "font-mono text-xs leading-relaxed text-muted-foreground";

function EndName({ conv, networks }: { conv: ForeignConversation | undefined; networks: readonly Network[] }) {
  if (!conv) return <span className="text-muted-foreground">a conversation that is gone</span>;
  const net = networks.find((n) => n.id === conv.network);
  return (
    <span className="inline-flex flex-wrap items-center gap-1.5">
      {net ? <NetChip chip={net.chip} /> : null}
      <span dir="auto" className="min-w-0 break-words text-foreground [unicode-bidi:isolate]">
        {convTitle(conv)}
      </span>
    </span>
  );
}

function EdgeCard({ edge, convs, networks, onError }: { edge: BridgeEdge; convs: ReadonlyMap<string, ForeignConversation>; networks: readonly Network[]; onError: (e: string | null) => void }) {
  const { dialog, attempt } = useConsentFlow();
  const [busy, setBusy] = useState(false);
  const src = convs.get(edge.src);
  const dst = convs.get(edge.dst);
  const consentPaused = edgeIsConsentPaused(edge);

  const patch = async (body: Parameters<Awaited<ReturnType<typeof getAdapter>>["bridge"]["patchEdge"]>[1], say: string) => {
    setBusy(true);
    onError(null);
    try {
      const adapter = await getAdapter();
      const out = await attempt((hash) => adapter.bridge.patchEdge(edge.id, hash ? { ...body, accept_warning: hash } : body));
      if (out.ok) {
        announce(say);
        void apiStore.refreshEdges();
      } else if (!out.cancelled) {
        if (redirectIfUnauthenticated(out.error)) return;
        onError(errorSentence(out.error, "The relay could not be changed."));
      }
    } finally {
      setBusy(false);
    }
  };
  const remove = async () => {
    setBusy(true);
    onError(null);
    try {
      const adapter = await getAdapter();
      await adapter.bridge.deleteEdge(edge.id);
      announce("Relay removed. Copies already queued stay; no new copy is made.");
      void apiStore.refreshEdges();
    } catch (err) {
      if (!redirectIfUnauthenticated(err)) onError(errorSentence(err, "The relay could not be removed."));
    } finally {
      setBusy(false);
    }
  };

  const fromMesh = src?.network === "meshtastic";
  return (
    <li className="flex flex-col gap-3 border-b border-border p-4 last:border-b-0" data-testid={`edge-${edge.id}`}>
      <div className="flex flex-wrap items-center gap-x-2 gap-y-1 font-mono text-sm">
        <EndName conv={src} networks={networks} />
        <span aria-label="copied to" className="text-muted-foreground">
          {"->"}
        </span>
        <EndName conv={dst} networks={networks} />
      </div>
      <p className={TEXT} data-testid="edge-state">
        {edgeStateText(edge)}
      </p>
      <p className={TEXT} data-testid="edge-counters">
        {edgeCountersText(edge)}
      </p>
      {edge.counters.shed > 0 ? <p className={TEXT}>Shed: the backlog was full, so the oldest copies were dropped and are not made again.</p> : null}
      <div className="flex flex-wrap items-center gap-3">
        {edge.paused ? (
          <Button size="sm" variant={consentPaused ? "primary" : "default"} arrow={false} loading={busy} onClick={() => void patch({ paused: false }, "Relay resumed.")} data-testid={`resume-${edge.id}`}>
            {consentPaused ? "review and resume" : "resume"}
          </Button>
        ) : (
          <Button size="sm" loading={busy} onClick={() => void patch({ paused: true }, "Relay paused.")} data-testid={`pause-${edge.id}`}>
            pause
          </Button>
        )}
        <Button size="sm" variant="destructive" confirm="Remove this relay" loading={busy} onClick={() => void remove()}>
          remove
        </Button>
      </div>
      <div className="flex flex-wrap items-start gap-4">
        {fromMesh ? (
          <Toggle pressed={edge.honour_ok_to_mqtt} onPressedChange={(next) => void patch({ honour_ok_to_mqtt: next }, next ? "Honouring the ok-to-MQTT bit." : "Not honouring the ok-to-MQTT bit.")} busy={busy} aria-describedby={`ok-${edge.id}`}>
            honour ok-to-MQTT <span aria-hidden="true">{edge.honour_ok_to_mqtt ? "[ON]" : "[OFF]"}</span>
          </Toggle>
        ) : null}
        <Segmented
          label="When a copy is too long"
          value={edge.oversize}
          onChange={(v) => void patch({ oversize: v }, v === "split" ? "Long copies are split into numbered parts." : "Long copies are not relayed.")}
          options={[
            { value: "refuse", label: "refuse" },
            { value: "split", label: "split" },
          ]}
        />
      </div>
      <p id={`ok-${edge.id}`} className={TEXT}>
        {fromMesh ? "Honouring ok-to-MQTT: a Meshtastic text whose sender did not set that bit is not relayed off the mesh and counts as too long to relay. Off by default. " : ""}
        A copy longer than the destination takes is never cut: refuse counts it, split sends numbered parts.
      </p>
      {dialog}
    </li>
  );
}

function CreateEdge({ convs, networks, onError }: { convs: ForeignConversation[]; networks: readonly Network[]; onError: (e: string | null) => void }) {
  const sources = convs.filter((c) => edgeEndpoint(c) && !edgeDestinationOnly(c));
  const targets = convs.filter(edgeEndpoint);
  const [src, setSrc] = useState("");
  const [dst, setDst] = useState("");
  const [oversize, setOversize] = useState<"refuse" | "split">("refuse");
  const [busy, setBusy] = useState(false);
  const { dialog, attempt } = useConsentFlow();

  const submit = async (e: FormEvent) => {
    e.preventDefault();
    if (!src || !dst || src === dst) return onError("Pick two different conversations.");
    setBusy(true);
    onError(null);
    try {
      const adapter = await getAdapter();
      const out = await attempt((hash) => adapter.bridge.createEdge({ src, dst, honour_ok_to_mqtt: false, oversize, ...(hash ? { accept_warning: hash } : {}) }));
      if (out.ok) {
        announce("Relay started. History is not copied; only new messages are.");
        setSrc("");
        setDst("");
        void apiStore.refreshEdges();
        void apiStore.refreshNetworks();
      } else if (!out.cancelled) {
        if (redirectIfUnauthenticated(out.error)) return;
        onError(errorSentence(out.error, "The relay could not be started."));
      }
    } finally {
      setBusy(false);
    }
  };
  const label = (c: ForeignConversation) => `${networks.find((n) => n.id === c.network)?.chip ?? "?"} · ${convTitle(c)}`;
  if (sources.length === 0 || targets.length < 2) return <p className={TEXT}>A relay needs two channels or recipient lists. They appear here as the networks report them.</p>;
  return (
    <form className="flex flex-col gap-3" onSubmit={(e) => void submit(e)} noValidate>
      <div className="grid gap-3 sm:grid-cols-2">
        <label className="flex flex-col gap-1">
          <span className="font-mono text-[10px] uppercase tracking-widest text-foreground">copy everything from</span>
          <select className={FIELD_CLASS} value={src} onChange={(e) => setSrc(e.target.value)} data-testid="edge-src">
            <option value="">choose a channel</option>
            {sources.map((c) => (
              <option key={c.id} value={c.id}>
                {label(c)}
              </option>
            ))}
          </select>
        </label>
        <label className="flex flex-col gap-1">
          <span className="font-mono text-[10px] uppercase tracking-widest text-foreground">to</span>
          <select className={FIELD_CLASS} value={dst} onChange={(e) => setDst(e.target.value)} data-testid="edge-dst">
            <option value="">choose a channel or recipient list</option>
            {targets
              .filter((c) => c.id !== src)
              .map((c) => (
                <option key={c.id} value={c.id}>
                  {label(c)}
                  {edgeDestinationOnly(c) ? " (recipient list)" : ""}
                </option>
              ))}
          </select>
        </label>
      </div>
      <Segmented
        label="When a copy is too long"
        value={oversize}
        onChange={setOversize}
        options={[
          { value: "refuse", label: "refuse" },
          { value: "split", label: "split" },
        ]}
      />
      <p className={`max-w-prose ${TEXT}`}>
        Only new messages are copied; history never is. Each copy leads with [name/TAG] so a reader knows who wrote it, and a copy is never relayed again. A direct message cannot be relayed. You will read the exact warning before it starts.
      </p>
      <div>
        <Button type="submit" size="sm" variant="primary" arrow={false} loading={busy} aria-disabled={!src || !dst || undefined} data-testid="edge-start">
          start a relay…
        </Button>
      </div>
      {dialog}
    </form>
  );
}

/** Relays that touch `network` (or all of them), with the form to start one. */
export function EdgesPanel({ network }: { network?: string }) {
  const edges = useEdges();
  const networks = useNetworks();
  const enabled = enabledNetworks(networks);
  const all = useConversationsOf(enabled.map((n) => n.id));
  const convs = Object.values(all).flat();
  const byId = new Map(convs.map((c) => [c.id, c] as const));
  const [error, setError] = useState<string | null>(null);
  const mine = (edges ?? []).filter((e) => !network || byId.get(e.src)?.network === network || byId.get(e.dst)?.network === network);
  if (edges === null) return null;
  const tooLong = mine.reduce((n, e) => n + e.counters.refused, 0);
  return (
    <Panel title="relays" suffix={network ?? "all"} flush>
      <div className="flex flex-col gap-1 border-b border-border px-4 py-3">
        <h2 className="font-mono text-[10px] uppercase tracking-widest text-foreground">Relays</h2>
        <p className={`max-w-prose ${TEXT}`}>
          A relay copies one conversation into another. It is one-way, and A to B plus B to C never gives A to C. Only you start one; the agent can only suggest it.
          {mine.length > 0 ? ` ${mine.length === 1 ? "1 relay" : `${mine.length} relays`}${tooLong > 0 ? `, ${tooLong} too long to relay` : ""}.` : ""}
        </p>
      </div>
      {mine.length === 0 ? (
        <p className={`px-4 py-3 ${TEXT}`}>No relays{network ? " on this network" : ""}.</p>
      ) : (
        <ul>
          {mine.map((e) => (
            <EdgeCard key={e.id} edge={e} convs={byId} networks={networks ?? []} onError={setError} />
          ))}
        </ul>
      )}
      {error ? (
        <p role="alert" className="border-t border-border px-4 py-2 font-mono text-xs text-destructive" data-testid="edges-error">
          {error}
        </p>
      ) : null}
      <div className="border-t border-border p-4">
        <CreateEdge convs={convs} networks={networks ?? []} onError={setError} />
      </div>
    </Panel>
  );
}

