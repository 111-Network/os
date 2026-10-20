// Optional components on Settings, Transport (os/60-transport/README.md §2.1): what each weighs before anything is installed, the licence
// notice and its acknowledgement (a 409 round-trip whose hash is recorded), enable and disable, and "remove files" with "also delete the
// cached download" as a separate choice. A component that is off has no process, listener, timer or open file.
import { useState } from "react";
import { announce } from "../../../app/LiveRegion";
import { Badge } from "../../../components/badge";
import { Button } from "../../../components/button";
import { CopyButton } from "../../../components/copy-button";
import { apiStore, getAdapter } from "../../../lib/api";
import { componentHasFiles, componentIsOn, componentWeightFacts, formatBytes } from "../../../lib/api/bridge";
import { networkLabel } from "../../../lib/api/networks";
import type { BridgeComponent, Network } from "../../../lib/api/types";
import { useConsentFlow } from "../../messages/net/consent";
import { plain } from "../../messages/net/model";
import { problemText } from "../model";
import { Feedback, FEEDBACK, Row } from "../row";
import { COMPONENTS_HELP } from "./model";

const refresh = () => {
  void apiStore.refreshComponents();
  void apiStore.refreshNetworks();
};

function ComponentCard({ c, networks }: { c: BridgeComponent; networks: readonly Network[] }) {
  const { dialog, attempt } = useConsentFlow();
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [alsoCache, setAlsoCache] = useState(false);
  const on = componentIsOn(c);
  const facts = componentWeightFacts(c);

  const run = async (fn: (a: Awaited<ReturnType<typeof getAdapter>>) => Promise<unknown>, say: string) => {
    setBusy(true);
    setError(null);
    try {
      await fn(await getAdapter());
      announce(say);
      refresh();
    } catch (err) {
      setError(problemText(err));
    } finally {
      setBusy(false);
    }
  };
  const enable = async () => {
    setBusy(true);
    setError(null);
    try {
      const adapter = await getAdapter();
      const out = await attempt((hash) => adapter.bridge.enableComponent(c.id, hash));
      if (out.ok) {
        announce(`${networkLabel(networks, c.adapter)} is on.`);
        refresh();
      } else if (!out.cancelled) setError(problemText(out.error));
    } finally {
      setBusy(false);
    }
  };

  return (
    <li className="flex flex-col gap-3 border border-border p-4" data-testid={`component-${c.id}`}>
      <div className="flex flex-wrap items-center gap-3">
        <span className="font-mono text-sm text-foreground">{networkLabel(networks, c.adapter)}</span>
        <Badge data-testid="component-state">{c.state}</Badge>
        {c.acknowledged ? <Badge>licence acknowledged</Badge> : null}
      </div>
      <ul className={`list-disc ps-5 text-muted-foreground ${FEEDBACK}`} data-testid="component-facts">
        {facts.map((f) => (
          <li key={f}>{f}</li>
        ))}
      </ul>
      <div className="flex flex-col gap-1 border border-border bg-secondary/20 p-3" data-testid="component-notice">
        <p className="font-mono text-[10px] uppercase tracking-widest text-muted-foreground">licence notice</p>
        <p className="max-w-prose font-mono text-xs leading-relaxed text-foreground">{c.license.notice}</p>
        <p className="flex flex-wrap items-center gap-2 break-all font-mono text-xs text-muted-foreground">
          {c.license.spdx} · read it at {plain(c.license.read_at, 120)}
          <CopyButton text={c.license.read_at} ariaLabel="Copy where to read the licence" />
        </p>
      </div>
      {c.reason_code || c.reason ? (
        <p className={`text-foreground ${FEEDBACK}`} data-testid="component-reason">
          {c.reason ? plain(c.reason, 200) : plain(c.reason_code ?? "", 80)}
        </p>
      ) : null}
      <div className="flex flex-wrap items-center gap-3">
        {on ? (
          <Button size="sm" loading={busy} onClick={() => void run((a) => a.bridge.disableComponent(c.id), "Turned off. Nothing of it runs.")} data-testid="component-disable">
            turn off
          </Button>
        ) : (
          <Button size="sm" variant="primary" arrow={false} loading={busy} onClick={() => void enable()} data-testid="component-enable">
            {c.acknowledged ? "turn on" : "read the notice and turn on"}
          </Button>
        )}
      </div>
      {!on && componentHasFiles(c) ? (
        <div className="flex flex-col gap-2 border-t border-border pt-3" data-testid="component-remove">
          <label className="flex min-h-11 items-center gap-3 font-mono text-xs text-foreground">
            <input type="checkbox" checked={alsoCache} onChange={(e) => setAlsoCache(e.target.checked)} className="h-4 w-4" />
            also delete the cached download ({formatBytes(c.cached_bytes)}); a later turn-on then has nothing to install from
          </label>
          <div>
            <Button
              size="sm"
              variant="destructive"
              confirm={alsoCache ? "Delete the installed files, this program's own data and the cached download" : "Delete the installed files and this program's own data"}
              loading={busy}
              onClick={() => void run((a) => a.bridge.removeComponentFiles(c.id, alsoCache), "Files removed.")}
            >
              remove files
            </Button>
          </div>
        </div>
      ) : null}
      {dialog}
      <Feedback text={error} problem />
    </li>
  );
}

export function ComponentsRow({ components, networks }: { components: readonly BridgeComponent[] | null; networks: readonly Network[] }) {
  return (
    <Row id="components" label="Optional components" help={COMPONENTS_HELP}>
      {components === null ? (
        <p className={`text-muted-foreground ${FEEDBACK}`}>Not reported yet.</p>
      ) : components.length === 0 ? (
        <p className={`text-muted-foreground ${FEEDBACK}`}>This release declares no optional components.</p>
      ) : (
        <ul className="flex flex-col gap-3" data-testid="components">
          {components.map((c) => (
            <ComponentCard key={c.id} c={c} networks={networks} />
          ))}
        </ul>
      )}
    </Row>
  );
}
