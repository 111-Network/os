// The `fabric` ("Messaging") service page body (os/5-messaging/README.md section 16): links, the home hub, what the node holds
// for others, the store gauge, and how delivery is reported. LIVE / NOT SET / ERROR itself is the registry's (the header);
// the set-up and fix buttons are the page's generic hand-off, which asks for `messaging.setup` / `messaging.fix`.
import { useEffect, useSyncExternalStore } from "react";
import { hrefTo } from "../../app/location";
import { ButtonLink } from "../../components/button";
import { Disclosure } from "../../components/disclosure";
import { Panel } from "../../components/panel";
import { SpecTable, type SpecRow } from "../../components/spec-table";
import { StatusDot, type DotState } from "../../components/status-dot";
import { hubSummary } from "../../lib/api/fabric";
import { useContacts, useFabric } from "../../lib/api/hooks";
import { cleanPeerText, MESSAGE_STATE_ORDER } from "../../lib/api/messages";
import { apiStore } from "../../lib/api/store";
import type { Contact, FabricHub, FabricLink, PairedHub, RelayStatus } from "../../lib/api/types";
import { useMode } from "../../lib/mode";
import { STATE_COPY } from "../messages/model";
import { custodyText, hubName, linkLabel, linkLine, pairFacts, profileSummary, refusedText, relayBadge, STORE_CLOSED_TEXT, STORE_FULL_TEXT, STORE_PRUNE_TEXT, tooLargeText } from "./fabric-model";
import { METERED_CAP_TEXT } from "./model";
import { PeerText } from "./shared";
import { StoreGauge } from "./store-gauge";

const TEXT = "max-w-prose font-mono text-xs leading-relaxed text-muted-foreground";

const linkDot = (l: Pick<FabricLink, "state">): DotState => (l.state === "up" ? "live" : l.state === "degraded" ? "error" : "unset");

/** "Home hub reachable: <name>": the name is the hub's own label when it is not aliased, so it is drawn isolated and cut. */
export function HubLine({ hub, contacts }: { hub: FabricHub; contacts: readonly Contact[] | null }) {
  const name = hubName(hub, contacts);
  return (
    <>
      {hubSummary(hub)}
      {hub.enrolled && name ? (
        <>
          {": "}
          <PeerText>{name}</PeerText>
        </>
      ) : null}
    </>
  );
}

const relaySlice = (): RelayStatus | null => apiStore.getState().relay;

/** The Layer 7 relay slice (paired hubs, forward counts, the gateway badge); null until loaded, and it stays null on a node without Layer 7. */
const useRelaySlice = (): RelayStatus | null => useSyncExternalStore(apiStore.subscribe, relaySlice, relaySlice);

/** A count from a snapshot: a non-negative whole number, else 0 (a missing or malformed field draws nothing). */
const count = (v: unknown): number => (typeof v === "number" && Number.isFinite(v) && v > 0 ? Math.floor(v) : 0);

const pairDot = (p: Pick<PairedHub, "reachable">): DotState => (p.reachable ? "live" : "unset");

/** One paired hub: its label (a peer string: cleaned, isolated) and the facts of `pairFacts`. */
export function PairedHubRow({ pair }: { pair: PairedHub }) {
  return (
    <li className="flex items-start gap-3 px-4 py-2.5 font-mono text-xs" data-testid="paired-hub">
      <StatusDot state={pairDot(pair)} label={false} name={pair.reachable ? "reachable" : "not reachable"} className="mt-1" />
      <div className="min-w-0 flex-1">
        <PeerText className="text-foreground">{cleanPeerText(pair.label).trim()}</PeerText>
        <p className="text-muted-foreground">{pairFacts(pair).join(" \u00b7 ")}</p>
      </div>
    </li>
  );
}

/**
 * The hub's relay facts (Layer 7 sections 3.8, 5.2, 5.8): the gateway badge with the metered line, each paired hub, and this session's
 * forward counts. Drawn for a hub, or for any node that already holds a pair or a count.
 */
export function RelayPanels({ relay, hubRole }: { relay: RelayStatus; hubRole: boolean }) {
  // Read defensively: a snapshot from a daemon build that omits `forward`, `gateway` or `paired_hubs` must draw less, never throw.
  const pairs = Array.isArray(relay.paired_hubs) ? relay.paired_hubs : [];
  const refused = count(relay.forward?.refused);
  const tooLarge = count(relay.forward?.too_large);
  const metered = relay.gateway?.metered_cap_reached === true;
  if (!hubRole && pairs.length === 0 && refused === 0 && tooLarge === 0) return null;
  return (
    <>
      <Panel title="gateway" suffix="this hub's own claim" bodyClassName="space-y-2">
        <h2 className="sr-only">Gateway</h2>
        <p className="max-w-prose font-mono text-sm leading-relaxed text-foreground" data-testid="gateway-badge">
          {relayBadge(relay)}
        </p>
        {metered ? (
          <p role="status" className="max-w-prose font-mono text-sm leading-relaxed text-foreground" data-testid="metered-cap">
            {METERED_CAP_TEXT}
          </p>
        ) : null}
      </Panel>

      <Panel title="paired hubs" suffix="hub to hub" flush>
        <h2 className="sr-only">Paired hubs</h2>
        {pairs.length === 0 ? (
          <p className="px-4 py-3 font-mono text-xs text-muted-foreground">No hub is paired with this one.</p>
        ) : (
          <ul className="divide-y divide-border">
            {pairs.map((p) => (
              <PairedHubRow key={p.node_id} pair={p} />
            ))}
          </ul>
        )}
        {refused > 0 || tooLarge > 0 ? (
          <div className="space-y-1 border-t border-border px-4 py-3" data-testid="forward-counts">
            {refused > 0 ? <p className={TEXT}>{refusedText(refused)}</p> : null}
            {tooLarge > 0 ? <p className={TEXT}>{tooLargeText(tooLarge)}</p> : null}
          </div>
        ) : null}
      </Panel>
    </>
  );
}

export function FabricDetails() {
  const fabric = useFabric();
  const relay = useRelaySlice();
  // Layer 7 snapshots load with this view only (never at start-up): a node without Layer 7 answers 404 and the slice stays null.
  useEffect(() => {
    void apiStore.refreshRelay();
  }, []);
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
    { label: "Home hub", value: <HubLine hub={hub} contacts={contacts} /> },
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

      {relay ? <RelayPanels relay={relay} hubRole={settings.hub_role} /> : null}

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
