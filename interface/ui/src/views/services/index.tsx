// Service page (spec 6.6): one registry record as a status header plus a body that depends on its state.
//   live       control panel built from `detail` (+ advanced: log tail, restart)
//   starting   the same panel with an honest "not answering yet" note
//   unset      what it is, what it needs, "Set up with the agent" (or nothing, when this device cannot run it)
//   error      reason, log lines, "Fix with the agent"
// The prompt text always comes from the one builder in lib/prompts.ts; nothing here is per-service.
import { useState } from "react";
import { useParams } from "wouter";
import { useNow } from "../../app/hooks";
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
import { useConnection, useDevices, useServices, useStatus } from "../../lib/api/hooks";
import type { DeviceRecord, ServiceRecord } from "../../lib/api/types";
import { useMode, Adv } from "../../lib/mode";
import { dotState } from "../../lib/nav";
import { agentHrefFor, buildPrompt, MAX_LOG_LINES, stagePrompt } from "../../lib/prompts";
import { fetchLogLines, LOG_POLL_MS, LogTail } from "./log";
import { flattenDetail, KIND_LABEL, RESTART_UNAVAILABLE, servicePageState, sinceText, type Handoff, type ServicePageState } from "./model";
import { RouteLink } from "./shared";

const TEXT = "font-mono text-xs leading-relaxed text-muted-foreground";
const LABEL = "font-mono text-[10px] uppercase tracking-widest text-muted-foreground";

// ------------------------------------------------------------------------------------ hand-off

function HandoffButton({ service, handoff, related }: { service: ServiceRecord; handoff: Handoff; related: DeviceRecord[] }) {
  const status = useStatus();
  const [, navigate] = useAppLocation();
  const [busy, setBusy] = useState(false);
  const [steps, setSteps] = useState<string | null>(null);

  const onClick = async () => {
    if (steps !== null) {
      setSteps(null);
      return;
    }
    setBusy(true);
    try {
      const logLines = await fetchLogLines(service.id, MAX_LOG_LINES);
      const text = buildPrompt(handoff.goal, { kind: "service", service, devices: related }, { status, logLines });
      if (handoff.mode === "agent") navigate(agentHrefFor(stagePrompt(text, service.plain_name, handoff.goal)));
      else setSteps(text);
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="space-y-3">
      <Button variant="primary" loading={busy} onClick={() => void onClick()} aria-expanded={handoff.mode === "steps" ? steps !== null : undefined}>
        {steps !== null ? "Hide steps" : handoff.label}
      </Button>
      {handoff.mode === "agent" ? (
        <p className={TEXT}>This opens the agent with a prepared message. Nothing is sent until you send it.</p>
      ) : (
        <p className={TEXT}>The agent is not running right now, so the steps are shown here instead.</p>
      )}
      {steps !== null ? (
        <div className="space-y-2">
          <pre
            tabIndex={0}
            className="max-h-96 overflow-auto whitespace-pre-wrap break-words border border-border bg-secondary/20 p-3 font-mono text-xs leading-relaxed text-muted-foreground focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-foreground focus-visible:outline-none"
          >
            {steps}
          </pre>
          <CopyButton text={steps} label="copy steps" />
        </div>
      ) : null}
    </div>
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

function Links({ page }: { page: ServicePageState }) {
  if (!page.kbPath && !page.embedUrl) return null;
  return (
    <div className="flex flex-wrap items-center gap-3">
      {page.kbPath ? <ButtonLink href={hrefTo(page.kbPath)}>Read the recipe</ButtonLink> : null}
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
      {page.log || page.restart ? (
        <section className="space-y-3">
          <h2 className={LABEL}>advanced</h2>
          {page.log ? <LogTail id={service.id} title={`${service.name} log`} lines={100} pollMs={LOG_POLL_MS} controls maxHeightClass="max-h-96" /> : null}
          {page.restart ? (
            <div className="flex flex-wrap items-center gap-3">
              <Button size="sm" variant="destructive" confirm={`Restart ${service.plain_name}`} disabled title={RESTART_UNAVAILABLE} aria-describedby="restart-note">
                Restart
              </Button>
              <span id="restart-note" className={TEXT}>
                {RESTART_UNAVAILABLE}
              </span>
            </div>
          ) : null}
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

  return (
    <div className="space-y-6">
      <SectionHeader level={1} kicker="service" title={service.plain_name} />

      <div className="space-y-2">
        <div className="flex flex-wrap items-center gap-x-4 gap-y-2">
          <StatusDot state={dotState(service.state, disconnected)} />
          <Badge>{KIND_LABEL[service.kind] ?? service.kind}</Badge>
          <span className={TEXT}>{sinceText(service.since, now)}</span>
        </div>
        <p className="font-mono text-sm leading-relaxed text-foreground">{service.reason}</p>
        {disconnected ? <p className={TEXT}>Last known state. The node is not answering right now.</p> : null}
        <p className={TEXT}>{service.description}</p>
      </div>

      {page.body === "live" ? (
        <>
          <ControlPanel service={service} devices={devices} />
          <Links page={page} />
        </>
      ) : null}

      {page.body === "starting" ? (
        <>
          <p className="font-mono text-xs leading-relaxed text-foreground">
            It was started a moment ago and is not answering its health check yet. This normally takes up to a couple of minutes.
          </p>
          <ControlPanel service={service} devices={devices} />
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
              <HandoffButton service={service} handoff={page.handoff} related={devices} />
            ) : (
              <p className={TEXT}>There is no setup step for this one. It turns live by itself once the condition above is met.</p>
            )}
          </Panel>
          <Links page={page} />
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
          <ControlPanel service={service} devices={devices} />
          {page.log && mode === "simple" ? (
            <Disclosure summary="Last log lines">
              <LogTail id={service.id} title={`${service.name} log`} lines={5} pollMs={0} />
            </Disclosure>
          ) : null}
          {page.handoff ? <HandoffButton service={service} handoff={page.handoff} related={devices} /> : null}
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
