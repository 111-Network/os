// Overview (spec 6.2): one verdict sentence, what needs the user, then the node at a glance.
import { useEffect, useMemo } from "react";
import { hrefTo, routeFragment, useAppLocation } from "../../app/location";
import { useNow } from "../../app/hooks";
import { Button } from "../../components/button";
import { CopyButton } from "../../components/copy-button";
import { Disclosure } from "../../components/disclosure";
import { JsonBlock } from "../../components/json-block";
import { MiniTerminal } from "../../components/mini-terminal";
import { Panel } from "../../components/panel";
import { SectionHeader } from "../../components/section-header";
import { SpecTable, type SpecRow } from "../../components/spec-table";
import { StatusDot } from "../../components/status-dot";
import { useActivity, useConnection, useDevices, useProposals, useServices, useStatus } from "../../lib/api/hooks";
import type { ServiceRecord } from "../../lib/api/types";
import { cx, FOCUS_RING } from "../../lib/cx";
import { Adv } from "../../lib/mode";
import { categoryLabel, dotState } from "../../lib/nav";
import { agentHrefFor, buildPrompt, stagePrompt } from "../../lib/prompts";
import { verdict, verdictDot } from "../../lib/verdict";
import { reasonText } from "../services/model";
import { RouteLink } from "../services/shared";
import { devicesSummary, needsYou, nearbySentence, nodeFacts, NEEDS_YOU_ID, recentActivity, type ChecklistRow, type NeedItem } from "./model";

const LABEL = "font-mono text-[10px] uppercase tracking-widest text-muted-foreground";
const TEXT = "font-mono text-xs leading-relaxed text-muted-foreground";

const KIND_LABEL: Record<NeedItem["kind"], string> = { proposal: "proposal", service: "service", device: "device" };

function ChecklistList({ rows }: { rows: ChecklistRow[] }) {
  return (
    <ul className="divide-y divide-border border border-border">
      {rows.map((r) => (
        <li key={r.id} className="flex items-start gap-3 px-3 py-2.5">
          <StatusDot state={r.state} label={false} name={r.title} className="mt-1" />
          <div className="min-w-0 flex-1 font-mono text-xs leading-relaxed">
            <span className="text-foreground">{r.title}</span>
            <span className="text-muted-foreground"> {"—"} {r.text}</span>
            {r.href && !r.done ? (
              <>
                {" "}
                <RouteLink to={r.href}>open</RouteLink>
              </>
            ) : null}
            {r.command ? (
              <div className="mt-2 flex flex-wrap items-center gap-2">
                <code className="whitespace-pre-wrap break-words border border-border bg-secondary/20 px-2 py-1 text-foreground">{r.command}</code>
                <CopyButton text={r.command} label="copy command" ariaLabel="Copy the identity backup command" />
              </div>
            ) : null}
          </div>
        </li>
      ))}
    </ul>
  );
}

function NeedsYouPanel() {
  const status = useStatus();
  const services = useServices();
  const devices = useDevices();
  const proposals = useProposals();
  const need = useMemo(() => needsYou(status, services, devices, proposals), [status, services, devices, proposals]);
  const done = need.checklist.filter((r) => r.done).length;
  const unknown = need.checklist.filter((r) => r.state === "unknown").length;
  // Until the derivable steps are done the checklist is the point; afterwards it folds away.
  const setupPending = need.checklist.some((r) => r.id !== "identity" && !r.done);

  // `#/overview#needs-you` is the header badge's target. The browser cannot scroll to a second `#`, so do it here.
  useEffect(() => {
    const go = () => {
      if (routeFragment(window.location.hash) !== NEEDS_YOU_ID) return;
      const el = document.getElementById(NEEDS_YOU_ID);
      if (!el) return;
      el.scrollIntoView({ block: "start" });
      el.focus({ preventScroll: true });
    };
    // After the shell's own scroll-to-top on a route change.
    const t = window.setTimeout(go, 0);
    window.addEventListener("hashchange", go);
    return () => {
      window.clearTimeout(t);
      window.removeEventListener("hashchange", go);
    };
  }, []);

  return (
    <Panel
      id={NEEDS_YOU_ID}
      role="region"
      aria-label="Needs you"
      tabIndex={-1}
      title="needs you"
      className={cx("scroll-mt-20 outline-none", FOCUS_RING)}
      bodyClassName="space-y-4"
    >
      <h2 className="sr-only">Needs you</h2>
      {need.items.length > 0 ? (
        <ul className="divide-y divide-border border border-border">
          {need.items.map((item) => (
            <li key={item.key} className="flex items-start gap-3 px-3 py-2.5 font-mono text-xs leading-relaxed">
              {item.kind === "proposal" ? (
                <span aria-hidden="true" className="mt-1 inline-block h-2 w-2 shrink-0 border border-foreground" />
              ) : (
                <StatusDot state="error" label={false} name={item.label} className="mt-1" />
              )}
              <div className="min-w-0 flex-1">
                <span className={LABEL}>{KIND_LABEL[item.kind]}</span>
                <div>
                  <RouteLink to={item.href} className="inline-flex min-h-11 items-center">
                    {item.label}
                  </RouteLink>
                </div>
                <div className="text-muted-foreground">{item.detail}</div>
              </div>
            </li>
          ))}
        </ul>
      ) : null}

      {setupPending ? (
        <div className="space-y-2">
          <p className={LABEL}>first-run checklist</p>
          <ChecklistList rows={need.checklist} />
        </div>
      ) : (
        <div className="space-y-1">
          {need.items.length === 0 ? <p className="font-mono text-sm text-foreground">Nothing needs you right now.</p> : null}
          <Disclosure summary={`First-run checklist \u00b7 ${done} of ${need.checklist.length} done${unknown > 0 ? ` \u00b7 ${unknown} unknown` : ""}`}>
            <ChecklistList rows={need.checklist} />
          </Disclosure>
        </div>
      )}
    </Panel>
  );
}

function ServiceCard({ service, disconnected }: { service: ServiceRecord; disconnected: boolean }) {
  return (
    <a href={hrefTo(`/services/${service.id}`)} className={cx("group block", FOCUS_RING)}>
      <Panel className="h-full transition-colors duration-200 group-hover:border-foreground" bodyClassName="space-y-2">
        <div className="flex items-start justify-between gap-3">
          <span className="min-w-0 font-mono text-sm text-foreground">{service.plain_name}</span>
          <StatusDot state={dotState(service.state, disconnected)} />
        </div>
        <p className={cx(TEXT, "line-clamp-2")}>{reasonText(service)}</p>
      </Panel>
    </a>
  );
}

function ServicesGrid() {
  const services = useServices();
  const { disconnected } = useConnection();
  if (services === null) return <p className={TEXT}>Waiting for the node to report its services.</p>;
  const here = services.filter((s) => s.can_run_here !== false);
  const elsewhere = services.filter((s) => s.can_run_here === false);
  const grid = "grid gap-3 md:grid-cols-2 lg:grid-cols-3";
  return (
    <>
      <div className={grid}>
        {here.map((s) => (
          <ServiceCard key={s.id} service={s} disconnected={disconnected} />
        ))}
      </div>
      {elsewhere.length > 0 ? (
        <Disclosure summary={`not available on this device (${elsewhere.length})`} className="mt-3">
          <div className={cx(grid, "pt-1")}>
            {elsewhere.map((s) => (
              <ServiceCard key={s.id} service={s} disconnected={disconnected} />
            ))}
          </div>
        </Disclosure>
      ) : null}
    </>
  );
}

export default function OverviewView() {
  const status = useStatus();
  const services = useServices();
  const devices = useDevices();
  const activity = useActivity();
  const { disconnected } = useConnection();
  const now = useNow(true, 15_000);
  const [, navigate] = useAppLocation();

  const v = useMemo(() => verdict(status, services, devices), [status, services, devices]);
  const facts = status ? nodeFacts(status, services, now) : null;
  const dev = devicesSummary(devices);

  const checkNode = () => {
    const text = buildPrompt("check", { kind: "node" }, { status });
    navigate(agentHrefFor(stagePrompt(text, "Overview", "check")));
  };

  const nodeRows: SpecRow[] = facts
    ? [
        { label: "Label", value: facts.label ?? "not available yet" },
        {
          label: "Node id",
          value: facts.bech32 ? (
            <span className="inline-flex flex-wrap items-center justify-end gap-2">
              <span className="break-all">{facts.bech32}</span>
              <CopyButton text={facts.bech32} label="copy" ariaLabel="Copy the node id" />
            </span>
          ) : (
            "not available yet"
          ),
        },
        { label: "Alias", value: facts.alias ?? "not set" },
        { label: "Category", value: facts.category ? categoryLabel(facts.category) : "unknown" },
        { label: "Version", value: facts.version },
        { label: "Uptime", value: facts.uptime },
        { label: "Updates", value: facts.update },
        { label: "Last update check", value: facts.lastCheck },
        { label: "Internet gateway", value: facts.gateway },
        {
          label: "Dashboard address",
          value:
            facts.addresses.length > 0 ? (
              <span className="flex flex-col items-end">
                {facts.addresses.map((u) => (
                  <span key={u} className="break-all">
                    {u}
                  </span>
                ))}
                <span className="text-muted-foreground">{facts.reach}</span>
              </span>
            ) : (
              "not available yet"
            ),
        },
      ]
    : [];

  return (
    <div className="space-y-6">
      <SectionHeader level={1} kicker="overview" title={facts?.name ?? "This node"} />

      <div className="flex items-start gap-3">
        <span aria-hidden="true" className="mt-1.5">
          <StatusDot state={verdictDot(v.tone, disconnected)} label={false} />
        </span>
        <div>
          <p className="font-mono text-sm leading-relaxed text-foreground">{v.sentence}</p>
          {disconnected ? <p className={cx(TEXT, "mt-1")}>Last known state. The node is not answering right now.</p> : null}
        </div>
      </div>

      <NeedsYouPanel />

      <div className="grid gap-3 md:grid-cols-2">
        <Panel title="nearby" bodyClassName="space-y-3">
          <h2 className="sr-only">Nearby</h2>
          <p className="font-mono text-sm leading-relaxed text-foreground">{nearbySentence(status)}</p>
          <Adv>
            <p className={TEXT}>The nearby-nodes table arrives with the relay and presence layers. Today the node only knows whether it has heard anyone.</p>
          </Adv>
          <RouteLink to="/nearby" className="inline-flex min-h-11 items-center font-mono text-xs">
            See who is nearby
          </RouteLink>
        </Panel>
        <Panel title="devices" bodyClassName="space-y-3">
          <h2 className="sr-only">Devices</h2>
          <p className="font-mono text-sm leading-relaxed text-foreground">{dev.text}</p>
          <RouteLink to="/devices" className="inline-flex min-h-11 items-center font-mono text-xs">
            Manage devices
          </RouteLink>
        </Panel>
      </div>

      <section aria-labelledby="overview-services" className="space-y-2">
        <h2 id="overview-services" className={LABEL}>
          services
        </h2>
        <ServicesGrid />
      </section>

      {facts ? (
        <Panel title="node" flush>
          <h2 className="sr-only">Node</h2>
          <SpecTable bare rows={nodeRows} />
        </Panel>
      ) : null}

      <section aria-labelledby="overview-activity" className="space-y-3">
        <h2 id="overview-activity" className={LABEL}>
          agent activity
        </h2>
        <MiniTerminal
          title="agent activity"
          lines={recentActivity(activity, 20)}
          follow
          emptyText="No agent activity yet. It shows up here when you chat with the agent or it works on a task."
        />
        <Button size="sm" onClick={checkNode}>
          Let the agent check this node
        </Button>
      </section>

      <Adv>
        <section className="space-y-3">
          <h2 className={LABEL}>advanced</h2>
          <JsonBlock value={status} summary="raw status (GET /api/v1/status)" />
          <MiniTerminal title="all agent events" lines={recentActivity(activity, 200)} maxHeightClass="max-h-96" emptyText="No agent events received on this page yet." />
        </section>
      </Adv>
    </div>
  );
}
