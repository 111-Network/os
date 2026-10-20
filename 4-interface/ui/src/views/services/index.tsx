// Service page (spec 6.6): one registry record as a status header plus a body that depends on its state.
//   live       control panel built from `detail`, Restart when the daemon supervises it (+ advanced: log tail)
//   starting   the same panel with an honest "not answering yet" note
//   unset      what it is, what it needs, "Set up with the agent" (or nothing, when this device cannot run it)
//   error      reason, log lines, Restart, "Fix with the agent" (and the Settings -> Restart node link when back-off is stuck)
// The prompt text always comes from the one builder in lib/prompts.ts; nothing here is per-service.
//
// Layer 5 (os/5-messaging/README.md section 13, 14, 16) adds to it without changing that shape:
//   a LIVE service with a hosted panel gets a Panel tab (panel.tsx) beside the details;
//   `fabric` ("Messaging") shows its links, hub, custody and store (fabric.tsx) in place of the raw detail table;
//   `meshtastic` adds the nodes heard and the honest copy about its panel (meshtastic.tsx).
import { useEffect, useId, useState, type ReactNode } from "react";
import { useParams } from "wouter";
import { redirectIfUnauthenticated } from "../../app/AuthGate";
import { useNow } from "../../app/hooks";
import { announce } from "../../app/LiveRegion";
import { hrefTo, useAppLocation } from "../../app/location";
import { Badge } from "../../components/badge";
import { Button, ButtonLink } from "../../components/button";
import { CopyButton } from "../../components/copy-button";
import { Disclosure } from "../../components/disclosure";
import { Empty } from "../../components/empty";
import { Panel } from "../../components/panel";
import { SectionHeader } from "../../components/section-header";
import { SpecTable, type SpecRow } from "../../components/spec-table";
import { StatusDot } from "../../components/status-dot";
import { getAdapter } from "../../lib/api";
import { useConnection, useDevices, usePanels, useServices, useServicesRev, useStatus } from "../../lib/api/hooks";
import { panelFor } from "../../lib/api/panels";
import type { DeviceRecord, HostedPanel, ServiceRecord } from "../../lib/api/types";
import { useMode, Adv } from "../../lib/mode";
import { dotState } from "../../lib/nav";
import { agentHrefFor, buildPrompt, MAX_LOG_LINES, stagePrompt } from "../../lib/prompts";
import { FabricDetails } from "./fabric";
import { fetchLogLines, LOG_POLL_MS, LogTail } from "./log";
import { MeshtasticDetails } from "./meshtastic";
import { PanelTab } from "./panel";
import {
  acceptRestart,
  beginRestart,
  flattenDetail,
  KIND_LABEL,
  reasonText,
  REQUEST_COPY_LABEL,
  REQUEST_HIDE_LABEL,
  requestNote,
  RESTART_GIVE_UP_MS,
  RESTART_IDLE,
  restartButtonLabel,
  restartErrorText,
  servicePageState,
  settleRestart,
  sinceText,
  type Handoff,
  type RestartPhase,
  type ServicePageState,
} from "./model";
import { RouteLink } from "./shared";
import { Tabs } from "./tabs";

const TEXT = "font-mono text-xs leading-relaxed text-muted-foreground";
const LABEL = "font-mono text-[10px] uppercase tracking-widest text-muted-foreground";

// ------------------------------------------------------------------------------------ hand-off

function HandoffButton({
  service,
  handoff,
  related,
  recipePath,
  children,
}: {
  service: ServiceRecord;
  handoff: Handoff;
  related: DeviceRecord[];
  /** Route of the KB recipe. With the agent down it is the primary action: it has the real steps. */
  recipePath: string | null;
  children?: ReactNode;
}) {
  const status = useStatus();
  const [, navigate] = useAppLocation();
  const [busy, setBusy] = useState(false);
  const [steps, setSteps] = useState<string | null>(null);
  const panelId = useId();
  const offline = handoff.mode === "request";
  const recipeFirst = offline && recipePath !== null;

  const onClick = async () => {
    if (steps !== null) {
      setSteps(null);
      return;
    }
    setBusy(true);
    try {
      const logLines = await fetchLogLines(service.id, MAX_LOG_LINES);
      const text = buildPrompt(handoff.goal, { kind: "service", service, devices: related }, { status, logLines });
      if (!offline) navigate(agentHrefFor(stagePrompt(text, service.plain_name, handoff.goal)));
      else setSteps(text);
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="space-y-3">
      <div className="flex flex-wrap items-start gap-3">
        {recipeFirst ? (
          <ButtonLink variant="primary" href={hrefTo(recipePath)}>
            Read the recipe
          </ButtonLink>
        ) : null}
        <Button
          variant={recipeFirst ? "default" : "primary"}
          loading={busy}
          onClick={() => void onClick()}
          aria-expanded={offline ? steps !== null : undefined}
          aria-controls={offline ? panelId : undefined}
        >
          {steps !== null ? REQUEST_HIDE_LABEL : handoff.label}
        </Button>
        {children}
      </div>
      {offline ? (
        <p className={TEXT}>{requestNote(recipePath !== null)}</p>
      ) : (
        <p className={TEXT}>This opens the agent with a prepared message. Nothing is sent until you send it.</p>
      )}
      <div id={panelId} hidden={steps === null}>
        {steps !== null ? (
          <div className="space-y-2">
            <pre
              tabIndex={0}
              className="max-h-96 overflow-auto whitespace-pre-wrap break-words border border-border bg-secondary/20 p-3 font-mono text-xs leading-relaxed text-muted-foreground focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-foreground focus-visible:outline-none"
            >
              {steps}
            </pre>
            <CopyButton text={steps} label={REQUEST_COPY_LABEL} />
          </div>
        ) : null}
      </div>
    </div>
  );
}

// -------------------------------------------------------------------------------------- restart

/**
 * Restart one supervised child. The button reads "Restarting…" from the click until the next service
 * list arrives (the node answers 202 at once; the new state comes as `services` frames).
 */
function RestartControl({ service }: { service: ServiceRecord }) {
  const rev = useServicesRev();
  const [phase, setPhase] = useState<RestartPhase>(RESTART_IDLE);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    setPhase((p) => settleRestart(p, rev));
  }, [rev]);
  useEffect(() => {
    if (phase.kind !== "waiting") return;
    const h = setTimeout(() => setPhase(RESTART_IDLE), RESTART_GIVE_UP_MS);
    return () => clearTimeout(h);
  }, [phase]);

  const restart = async () => {
    setError(null);
    setPhase(beginRestart(rev));
    try {
      const adapter = await getAdapter();
      await adapter.restartService(service.id);
      setPhase(acceptRestart);
      announce(`Restarting ${service.plain_name}.`);
    } catch (err) {
      if (redirectIfUnauthenticated(err)) return;
      setPhase(RESTART_IDLE);
      setError(restartErrorText(err));
    }
  };

  return (
    <span className="inline-flex flex-col items-start gap-2">
      <Button
        confirm={`Restart ${service.plain_name}`}
        loading={phase.kind !== "idle"}
        onClick={() => void restart()}
      >
        {restartButtonLabel(phase)}
      </Button>
      {error ? (
        <span role="alert" className="font-mono text-xs leading-relaxed text-destructive">
          Problem: {error}
        </span>
      ) : null}
    </span>
  );
}

// ---------------------------------------------------------------------------------- body pieces

function DeviceLinks({ devices }: { devices: DeviceRecord[] }) {
  return (
    <span className="flex flex-col items-end">
      {devices.map((d) => (
        <RouteLink key={d.id} to={`/devices/${encodeURIComponent(d.id)}`}>
          {d.alias ?? d.name}
        </RouteLink>
      ))}
    </span>
  );
}

function ControlPanel({ service, devices }: { service: ServiceRecord; devices: DeviceRecord[] }) {
  const { mode } = useMode();
  const now = useNow(true, 15_000);
  const rows: SpecRow[] = flattenDetail(service.detail, now).map((r) => ({ label: r.label, value: r.value }));
  if (devices.length > 0) rows.push({ label: devices.length === 1 ? "Device" : "Devices", value: <DeviceLinks devices={devices} /> });
  rows.push(
    { label: "Technical name", value: service.name, minMode: "advanced" },
    { label: "Registry reason code", value: service.reason_code, minMode: "advanced" },
  );
  return (
    <Panel title={service.plain_name} suffix="control panel" flush>
      <h2 className="sr-only">Control panel</h2>
      <SpecTable bare mode={mode} rows={rows} />
    </Panel>
  );
}

/**
 * What the page shows about a running service. The generic control panel (built from `detail`) for most; for `fabric` the
 * messaging details with the raw registry table kept for Advanced; `meshtastic` adds its nodes-heard and honest copy.
 */
function ServiceDetails({ service, devices, panel }: { service: ServiceRecord; devices: DeviceRecord[]; panel: HostedPanel | null }) {
  if (service.id === "fabric") {
    return (
      <>
        <FabricDetails />
        <Adv>
          <ControlPanel service={service} devices={devices} />
        </Adv>
      </>
    );
  }
  return (
    <>
      <ControlPanel service={service} devices={devices} />
      {service.id === "meshtastic" ? <MeshtasticDetails panel={panel} /> : null}
    </>
  );
}

type TabId = "details" | "panel";
const TAB_DEFS = [
  { id: "details", label: "Details" },
  { id: "panel", label: "Panel" },
] as const;

/** A LIVE service with a hosted panel: Details | Panel. The panel is mounted when its tab is first opened and kept after that. */
function WithPanelTab({ service, panel, children }: { service: ServiceRecord; panel: HostedPanel; children: ReactNode }) {
  const [tab, setTab] = useState<TabId>("details");
  const [opened, setOpened] = useState(false);
  const choose = (next: TabId) => {
    setTab(next);
    if (next === "panel") setOpened(true);
  };
  return (
    <Tabs<TabId> label={`${service.plain_name} sections`} tabs={TAB_DEFS} value={tab} onChange={choose}>
      {(id) => (id === "details" ? children : opened ? <PanelTab service={service} panel={panel} /> : null)}
    </Tabs>
  );
}

function Links({ page, skipRecipe = false }: { page: ServicePageState; skipRecipe?: boolean }) {
  const recipe = skipRecipe ? null : page.kbPath;
  if (!recipe && !page.embedUrl) return null;
  return (
    <div className="flex flex-wrap items-center gap-3">
      {recipe ? <ButtonLink href={hrefTo(recipe)}>Read the recipe</ButtonLink> : null}
      {page.embedUrl ? (
        <ButtonLink href={page.embedUrl} external>
          Open full client
        </ButtonLink>
      ) : null}
    </div>
  );
}

function AdvancedTools({ service, page }: { service: ServiceRecord; page: ServicePageState }) {
  return (
    <Adv>
      {page.log ? (
        <section className="space-y-3">
          <h2 className={LABEL}>advanced</h2>
          <LogTail id={service.id} title={`${service.name} log`} lines={100} pollMs={LOG_POLL_MS} controls maxHeightClass="max-h-96" />
        </section>
      ) : null}
    </Adv>
  );
}

// -------------------------------------------------------------------------------------- the page

function ServicePage({ service }: { service: ServiceRecord }) {
  const services = useServices();
  const allDevices = useDevices();
  const { disconnected } = useConnection();
  const { mode } = useMode();
  const now = useNow(true, 15_000);
  const agentLive = services?.find((s) => s.id === "agent")?.state === "live";
  const page = servicePageState(service, agentLive);
  const devices = (allDevices ?? []).filter((d) => d.service_id === service.id);
  const panels = usePanels();
  // The record on the service is the registry's; the `panels` list is the fresher state of the same panel (SSE `panels`).
  const panel: HostedPanel | null = page.panelTab ? (panelFor(panels, service.id) ?? service.panel ?? null) : null;

  const live = (
    <>
      <ServiceDetails service={service} devices={devices} panel={panel} />
      {page.restart ? (
        <div>
          <RestartControl service={service} />
        </div>
      ) : null}
      <Links page={page} />
    </>
  );

  return (
    <div className="space-y-6">
      <SectionHeader level={1} kicker="service" title={service.plain_name} />

      <div className="space-y-2">
        <div className="flex flex-wrap items-center gap-x-4 gap-y-2">
          <StatusDot state={dotState(service.state, disconnected)} />
          <Badge>{KIND_LABEL[service.kind] ?? service.kind}</Badge>
          <span className={TEXT}>{sinceText(service.since, now)}</span>
        </div>
        <p className="font-mono text-sm leading-relaxed text-foreground">{reasonText(service)}</p>
        {disconnected ? <p className={TEXT}>Last known state. The node is not answering right now.</p> : null}
        <p className={TEXT}>{service.description}</p>
      </div>

      {page.body === "live" ? (
        panel ? (
          <WithPanelTab service={service} panel={panel}>
            <div className="flex flex-col gap-6">{live}</div>
          </WithPanelTab>
        ) : (
          live
        )
      ) : null}

      {page.body === "starting" ? (
        <>
          <p className="font-mono text-xs leading-relaxed text-foreground">
            It was started a moment ago and is not answering its health check yet. This normally takes up to a couple of minutes.
          </p>
          <ServiceDetails service={service} devices={devices} panel={null} />
          <Links page={page} />
        </>
      ) : null}

      {page.body === "unset" ? (
        <>
          <Panel title={service.plain_name} suffix="setup" bodyClassName="space-y-4">
            <h2 className="sr-only">Setup</h2>
            <SpecTable
              bare
              mode={mode}
              rows={[
                ...(service.requires ? [{ label: "Needs", value: service.requires }] : []),
                ...(devices.length > 0 ? [{ label: devices.length === 1 ? "Device" : "Devices", value: <DeviceLinks devices={devices} /> }] : []),
              ]}
            />
            {page.handoff ? (
              <HandoffButton service={service} handoff={page.handoff} related={devices} recipePath={page.kbPath} />
            ) : (
              <p className={TEXT}>There is no setup step for this one. It turns live by itself once the condition above is met.</p>
            )}
          </Panel>
          {service.id === "fabric" ? <FabricDetails /> : null}
          <Links page={page} skipRecipe={page.handoff?.mode === "request" && page.kbPath !== null} />
        </>
      ) : null}

      {page.body === "unavailable" ? (
        <>
          <Empty kicker="not available on this device" title="This device cannot run it">
            {service.requires ? `It needs: ${service.requires}` : "It needs hardware or software this device does not have."}
          </Empty>
          <Links page={page} />
        </>
      ) : null}

      {page.body === "error" ? (
        <>
          <ServiceDetails service={service} devices={devices} panel={null} />
          {page.log && mode === "simple" ? (
            <Disclosure summary="Last log lines">
              <LogTail id={service.id} title={`${service.name} log`} lines={5} pollMs={0} />
            </Disclosure>
          ) : null}
          {page.handoff ? (
            <HandoffButton service={service} handoff={page.handoff} related={devices} recipePath={page.kbPath}>
              {page.restart ? <RestartControl service={service} /> : null}
              {page.nodeRestartLink ? <ButtonLink href={hrefTo("/settings")}>Settings {"->"} Restart node</ButtonLink> : null}
            </HandoffButton>
          ) : page.restart ? (
            <div>
              <RestartControl service={service} />
            </div>
          ) : null}
          <Links page={page} />
        </>
      ) : null}

      <AdvancedTools service={service} page={page} />
    </div>
  );
}

export default function ServicesView() {
  const { id = "" } = useParams<{ id: string }>();
  const services = useServices();
  if (services === null) {
    return (
      <p role="status" className={TEXT}>
        Reading the service registry{"…"}
      </p>
    );
  }
  const service = services.find((s) => s.id === id);
  if (!service) {
    return (
      <Empty kicker="404" title="No such service" action={<ButtonLink href={hrefTo("/overview")}>Back to Overview</ButtonLink>}>
        {`This node does not have a service called “${id}”.`}
      </Empty>
    );
  }
  return <ServicePage service={service} />;
}
