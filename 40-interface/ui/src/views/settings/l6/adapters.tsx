// Adapters on and off, and the preferred order (os/60-transport/README.md §2.1, §10, §11). Meshtastic is part of 111; the others are
// optional components, and turning one on shows its licence notice first (a 409 round-trip) and installs from the install-time cache.
import { useState } from "react";
import { announce } from "../../../app/LiveRegion";
import { Button } from "../../../components/button";
import { Toggle } from "../../../components/toggle";
import { apiStore, getAdapter } from "../../../lib/api";
import { componentWeightFacts } from "../../../lib/api/bridge";
import { networkLabel } from "../../../lib/api/networks";
import type { BridgeSettings, Network } from "../../../lib/api/types";
import { useConsentFlow } from "../../messages/net/consent";
import { networkStateLine, networkStateWord } from "../../messages/net/model";
import { NetChip } from "../../messages/net/glyph";
import { problemText } from "../model";
import { Feedback, FEEDBACK, Row } from "../row";
import { ADAPTERS_HELP, FABRIC_ID, FABRIC_LABEL, ORDER_HELP, moveItem, orderWithAll } from "./model";

const refresh = () => {
  void apiStore.refreshNetworks();
  void apiStore.refreshComponents();
  void apiStore.refreshBridgeSettings();
};

function AdapterRow({ n }: { n: Network }) {
  const { dialog, attempt } = useConsentFlow();
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const on = n.state !== "disabled";
  const line = networkStateLine(n);
  const facts = n.component && !on ? componentWeightFacts(n.component) : [];

  const press = async (next: boolean) => {
    setBusy(true);
    setError(null);
    try {
      const adapter = await getAdapter();
      const c = n.component;
      if (c) {
        if (next) {
          const out = await attempt((hash) => adapter.bridge.enableComponent(c.id, hash));
          if (!out.ok) {
            if (!out.cancelled) setError(problemText(out.error));
            return;
          }
          announce(`${n.label} is on.`);
        } else {
          await adapter.bridge.disableComponent(c.id);
          announce(`${n.label} is off. Nothing of it runs.`);
        }
      } else {
        await adapter.bridge.patchSettings({ adapters: { [n.id]: next } });
        announce(`${n.label} is ${next ? "on" : "off"}.`);
      }
      refresh();
    } catch (err) {
      setError(problemText(err));
    } finally {
      setBusy(false);
    }
  };

  return (
    <Row id={`adapter-${n.id.replace(/[^a-z0-9]/g, "-")}`} label={n.label} help={n.component ? "An optional program. Off means no process, listener or timer." : "Part of 111. Costs nothing until a board is bound."}>
      <div className="flex flex-col gap-3">
        <div className="flex flex-wrap items-center gap-3">
          <NetChip chip={n.chip} />
          <Toggle pressed={on} onPressedChange={(next) => void press(next)} busy={busy}>
            {n.label} <span aria-hidden="true">{on ? "[ON]" : "[OFF]"}</span>
          </Toggle>
          <span className="font-mono text-[10px] uppercase tracking-widest text-muted-foreground" data-testid={`state-${n.id}`}>
            {networkStateWord(n.state)}
          </span>
        </div>
        {line ? <p className={`max-w-prose text-muted-foreground ${FEEDBACK}`}>{line}</p> : null}
        {facts.length > 0 ? (
          <ul className={`list-disc ps-5 text-muted-foreground ${FEEDBACK}`} data-testid={`weight-${n.id}`}>
            {facts.map((f) => (
              <li key={f}>{f}</li>
            ))}
          </ul>
        ) : null}
        {dialog}
        <Feedback text={error} problem />
      </div>
    </Row>
  );
}

export function AdapterRows({ networks }: { networks: readonly Network[] }) {
  return (
    <>
      <Row id="adapters" label="Networks" help={ADAPTERS_HELP}>
        <p className={`text-muted-foreground ${FEEDBACK}`}>{networks.length === 0 ? "This node lists no foreign networks." : `${networks.length} ${networks.length === 1 ? "network" : "networks"} below.`}</p>
      </Row>
      {networks.map((n) => (
        <AdapterRow key={n.id} n={n} />
      ))}
    </>
  );
}

export function OrderRow({ settings, networks }: { settings: BridgeSettings | null; networks: readonly Network[] }) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const order = orderWithAll(settings?.order ?? [], [FABRIC_ID, ...networks.map((n) => n.id)]);
  const label = (id: string) => (id === FABRIC_ID ? FABRIC_LABEL : networkLabel(networks, id));

  const move = async (i: number, dir: -1 | 1) => {
    const next = moveItem(order, i, dir);
    if (next === order) return;
    setBusy(true);
    setError(null);
    try {
      const adapter = await getAdapter();
      apiStore.adoptBridgeSettings(await adapter.bridge.patchSettings({ order: next }));
      announce(`${label(order[i]!)} moved ${dir < 0 ? "up" : "down"}.`);
      void apiStore.refreshNetworks();
    } catch (err) {
      setError(problemText(err));
    } finally {
      setBusy(false);
    }
  };

  return (
    <Row id="order" label="Preferred order" help={ORDER_HELP}>
      <div className="flex flex-col gap-2">
        <ol className="border border-border" data-testid="order-list">
          {order.map((id, i) => (
            <li key={id} className="flex min-h-11 items-center justify-between gap-3 border-b border-border px-3 py-1 font-mono text-sm last:border-b-0" data-testid={`order-${id}`}>
              <span className="min-w-0 break-words text-foreground">
                {i + 1}. {label(id)}
              </span>
              <span className="flex shrink-0 gap-1">
                <Button size="sm" aria-label={`Move ${label(id)} up`} aria-disabled={busy || i === 0 || undefined} onClick={() => void move(i, -1)}>
                  up
                </Button>
                <Button size="sm" aria-label={`Move ${label(id)} down`} aria-disabled={busy || i === order.length - 1 || undefined} onClick={() => void move(i, 1)}>
                  down
                </Button>
              </span>
            </li>
          ))}
        </ol>
        <Feedback text={error} problem />
      </div>
    </Row>
  );
}
