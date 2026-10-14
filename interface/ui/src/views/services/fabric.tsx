// The `fabric` ("Messaging") service page body (os/messaging/README.md section 16): links, the home hub, what the node holds
// for others, the store gauge, and how delivery is reported. LIVE / NOT SET / ERROR itself is the registry's (the header);
// the set-up and fix buttons are the page's generic hand-off, which asks for `messaging.setup` / `messaging.fix`.
import { hrefTo } from "../../app/location";
import { ButtonLink } from "../../components/button";
import { Disclosure } from "../../components/disclosure";
import { Panel } from "../../components/panel";
import { SpecTable, type SpecRow } from "../../components/spec-table";
import { StatusDot, type DotState } from "../../components/status-dot";
import { useContacts, useFabric } from "../../lib/api/hooks";
import { cleanPeerText, MESSAGE_STATE_ORDER } from "../../lib/api/messages";
import type { FabricLink } from "../../lib/api/types";
import { useMode } from "../../lib/mode";
import { STATE_COPY } from "../messages/model";
import { custodyText, hubLine, hubName, linkLabel, linkLine, profileSummary, STORE_CLOSED_TEXT, STORE_FULL_TEXT, STORE_PRUNE_TEXT } from "./fabric-model";
import { StoreGauge } from "./store-gauge";

const TEXT = "max-w-prose font-mono text-xs leading-relaxed text-muted-foreground";

const linkDot = (l: Pick<FabricLink, "state">): DotState => (l.state === "up" ? "live" : l.state === "degraded" ? "error" : "unset");

export function FabricDetails() {
  const fabric = useFabric();
  const contacts = useContacts();
  const { mode } = useMode();
  const advanced = mode === "advanced";

  if (fabric === null) {
    return (
      <p role="status" className={TEXT}>
        The node has not reported its messaging state yet.
      </p>
    );
  }
  const { hub, links, custody, store, settings } = fabric;
  const name = hubName(hub, contacts);

  const linkRows: SpecRow[] =
    links.length === 0
      ? [{ label: "Links", value: "none yet" }]
      : links.map((l) => ({
          label: linkLabel(l.kind),
          value: (
            <span className="flex flex-col items-end gap-0.5">
              <span className="inline-flex items-center gap-2">
                <StatusDot state={linkDot(l)} label={false} name={linkLabel(l.kind)} />
                {linkLine(l)}
              </span>
              <span className="text-muted-foreground">{profileSummary(l.profile, advanced)}</span>
            </span>
          ),
        }));

  const hubRows: SpecRow[] = [
    { label: "Home hub", value: hubLine(hub, contacts) },
    ...(hub.enrolled && hub.address ? [{ label: "Hub address", value: <span className="break-all">{cleanPeerText(hub.address)}</span> }] : []),
    ...(settings.hub_role ? [{ label: "Nodes using this hub", value: String(hub.members) }] : []),
    ...(hub.offer_active ? [{ label: "Enrollment code", value: "one is outstanding (Settings shows how to dismiss it)" }] : []),
    { label: "Mailbox position", value: String(hub.cursor), minMode: "advanced" as const },
    ...(name && hub.node_id ? [{ label: "Hub node id", value: <span className="break-all">{hub.node_id}</span>, minMode: "advanced" as const }] : []),
  ];

  return (
    <div className="flex flex-col gap-6" data-testid="fabric-details">
      <Panel title="links" suffix="how this node reaches others" flush>
        <h2 className="sr-only">Links</h2>
        <SpecTable bare mode={mode} rows={linkRows} />
      </Panel>

      <Panel title="home hub" suffix={settings.hub_role ? "hub role on" : "member"} flush>
        <h2 className="sr-only">Home hub</h2>
        <SpecTable bare mode={mode} rows={hubRows} />
        <div className="flex flex-wrap items-center gap-3 border-t border-border px-4 py-3">
          <ButtonLink size="sm" href={hrefTo("/settings/l5")}>
            {hub.enrolled || settings.hub_role ? "Messaging settings" : "Enroll with a hub"}
          </ButtonLink>
          <ButtonLink size="sm" href={hrefTo("/messages/contacts")}>
            Contacts
          </ButtonLink>
        </div>
      </Panel>

      <Panel title="mail held for others" suffix="custody" bodyClassName="space-y-2">
        <h2 className="sr-only">Mail held for others</h2>
        <p className="max-w-prose font-mono text-sm leading-relaxed text-foreground">{custodyText(custody, settings.hub_role)}</p>
      </Panel>

      <Panel title="store" suffix="encrypted" bodyClassName="space-y-3">
        <h2 className="sr-only">Store</h2>
        {store.open ? <StoreGauge store={store} /> : <p className="font-mono text-sm text-foreground">{STORE_CLOSED_TEXT}</p>}
        {store.store_full ? (
          <p role="status" className="max-w-prose font-mono text-sm leading-relaxed text-foreground" data-testid="store-full">
            {STORE_FULL_TEXT}
          </p>
        ) : null}
        <p className={TEXT}>{STORE_PRUNE_TEXT}</p>
      </Panel>

      <Disclosure summary="How delivery is reported" defaultOpen={false}>
        <dl className="flex flex-col gap-2" data-testid="state-legend">
          {MESSAGE_STATE_ORDER.map((s) => (
            <div key={s} className="flex flex-col gap-0.5 sm:flex-row sm:gap-3">
              <dt className="w-24 shrink-0 font-mono text-xs text-foreground">{s}</dt>
              <dd className={TEXT}>{STATE_COPY[s]}</dd>
            </div>
          ))}
        </dl>
      </Disclosure>

      <div className="flex flex-wrap items-center gap-3">
        <ButtonLink href={hrefTo("/messages")}>Open Messages</ButtonLink>
      </div>
    </div>
  );
}
