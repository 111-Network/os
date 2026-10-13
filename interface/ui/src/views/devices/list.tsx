// Devices list (spec 6.3): everything attached to or paired with this node, grouped by kind.
import type { ReactNode } from "react";
import { useMediaQuery, useNow } from "../../app/hooks";
import { hrefTo } from "../../app/location";
import { Badge } from "../../components/badge";
import { ButtonLink } from "../../components/button";
import { DataTable, type DataColumn } from "../../components/data-table";
import { Empty } from "../../components/empty";
import { Panel } from "../../components/panel";
import { SectionHeader } from "../../components/section-header";
import { StatusDot } from "../../components/status-dot";
import { useApiMeta, useConnection, useDevices, useServices, useStatus } from "../../lib/api/hooks";
import type { DeviceRecord } from "../../lib/api/types";
import {
  agoText,
  confidenceLabel,
  deviceDot,
  displayName,
  drivenBy,
  groupByKind,
  isThisBrowser,
  kindTransport,
  registryPrompt,
  stateLabel,
  summary,
} from "../../lib/devices";
import { AgentButton, useAgentReachable } from "./agent-button";
import { useOwnDeviceId, type OwnDeviceId } from "./own-device";

export const detailHref = (id: string): string => hrefTo(`/devices/${encodeURIComponent(id)}`);

/**
 * Every cell of a row links to the detail page, so the whole row is a click target. Only the first
 * link is in the tab order and in the accessibility tree; the others are mouse and touch conveniences.
 */
function RowLink({ d, first, children }: { d: DeviceRecord; first?: boolean; children: ReactNode }) {
  return (
    <a
      href={detailHref(d.id)}
      tabIndex={first ? undefined : -1}
      aria-hidden={first ? undefined : true}
      className="-mx-4 -my-2.5 flex min-h-11 flex-col justify-center px-4 py-2.5 hover:text-foreground focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-foreground focus-visible:outline-none"
    >
      {children}
    </a>
  );
}

/** Below this width four columns do not fit: the other three fold into a second line of the name cell. */
const WIDE_QUERY = "(min-width: 640px)";

function columns(nodeDisconnected: boolean, now: number, wide: boolean, ownId: OwnDeviceId): DataColumn<DeviceRecord>[] {
  const name: DataColumn<DeviceRecord> = {
    key: "name",
    header: "Device",
    cell: (d) => (
      <RowLink d={d} first>
        <span className="flex items-center gap-2">
          <StatusDot state={deviceDot(d.state, nodeDisconnected)} label={false} />
          <span className="min-w-0 break-words">{displayName(d)}</span>
          {confidenceLabel(d) === "likely" ? <Badge className="shrink-0">likely</Badge> : null}
          {isThisBrowser(d, ownId) ? <Badge className="shrink-0">this browser</Badge> : null}
        </span>
        <span className="ps-4 text-[11px] leading-snug text-muted-foreground">
          {stateLabel(d.state, nodeDisconnected)}
          {(d.state === "needs_setup" || d.state === "error") && d.reason ? ` · ${d.reason}` : ""}
        </span>
        {wide ? null : (
          <span className="ps-4 text-[11px] leading-snug text-muted-foreground">
            {kindTransport(d)} · driven by {drivenBy(d)} · seen {agoText(d.last_seen, now)}
          </span>
        )}
      </RowLink>
    ),
  };
  if (!wide) return [name];
  return [
    name,
    {
      key: "kind",
      header: "Kind",
      muted: true,
      cell: (d) => (
        <RowLink d={d}>
          <span className="whitespace-nowrap">{kindTransport(d)}</span>
        </RowLink>
      ),
    },
    {
      key: "driver",
      header: "Driven by",
      muted: true,
      cell: (d) => (
        <RowLink d={d}>
          <span className="whitespace-nowrap">{drivenBy(d)}</span>
        </RowLink>
      ),
    },
    {
      key: "seen",
      header: "Last seen",
      muted: true,
      cell: (d) => (
        <RowLink d={d}>
          <span className="whitespace-nowrap">{agoText(d.last_seen, now)}</span>
        </RowLink>
      ),
    },
  ];
}

export default function DeviceList() {
  const devices = useDevices();
  const services = useServices();
  const status = useStatus();
  const { disconnected } = useConnection();
  const { ready } = useApiMeta();
  const reachable = useAgentReachable();
  const ownId = useOwnDeviceId();
  const now = useNow(true, 15_000);
  const wide = useMediaQuery(WIDE_QUERY);

  const addLink = (
    <ButtonLink href={hrefTo("/devices/add")} size="sm">
      Add device
    </ButtonLink>
  );

  if (devices === null) {
    return (
      <div className="flex flex-col gap-6">
        <SectionHeader kicker="devices" title="Devices" level={1} end={addLink} />
        {ready ? (
          <Empty kicker="unavailable" title="The device list could not be loaded">
            The node did not answer. The list appears here as soon as it does.
          </Empty>
        ) : (
          <p role="status" className="font-mono text-xs text-muted-foreground">
            Loading devices
          </p>
        )}
      </div>
    );
  }

  const groups = groupByKind(devices);
  const sum = summary(devices);
  const cols = columns(disconnected, now, wide, ownId);

  return (
    <div className="flex flex-col gap-6">
      <SectionHeader kicker="devices" title="Devices" level={1} end={addLink} />

      <div className="flex flex-col gap-3">
        <p className="font-mono text-sm text-foreground">{sum.sentence}</p>
        <p className="max-w-prose font-mono text-xs leading-relaxed text-muted-foreground">
          The agent looks at everything connected or paired, works out what each thing is, and gives you the steps or a proposal for
          what is missing.
        </p>
        <AgentButton
          label="Connect with the agent"
          from="Devices"
          goal="connect"
          reachable={reachable}
          prompt={() => registryPrompt({ devices, services: services ?? [], status, facts: true })}
        />
      </div>

      {devices.length === 0 ? (
        <Empty
          title="Nothing connected yet."
          action={
            <ButtonLink href={hrefTo("/devices/add")} size="sm">
              Add device
            </ButtonLink>
          }
        >
          Plug in a LoRa board, or pair a phone from Add device.
        </Empty>
      ) : (
        <div className="flex flex-col gap-4">
          {groups.map((g) => (
            <Panel key={g.kind} title={g.title} flush bodyClassName="[&>div]:border-0">
              <DataTable caption={g.title} columns={cols} rows={g.devices} rowKey={(d) => d.id} />
            </Panel>
          ))}
        </div>
      )}
    </div>
  );
}
