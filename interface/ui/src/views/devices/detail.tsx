// Device detail (spec 6.3): inline rename, spec table, state and reason, the agent hand-off, raw facts, Forget.
import { useEffect, useRef, useState, type FormEvent, type ReactNode } from "react";
import { navigate } from "wouter/use-hash-location";
import { useNow } from "../../app/hooks";
import { announce } from "../../app/LiveRegion";
import { hrefTo } from "../../app/location";
import { redirectIfUnauthenticated } from "../../app/AuthGate";
import { Badge } from "../../components/badge";
import { Button, ButtonLink } from "../../components/button";
import { CopyButton } from "../../components/copy-button";
import { Empty } from "../../components/empty";
import { Input } from "../../components/input";
import { JsonBlock } from "../../components/json-block";
import { Panel } from "../../components/panel";
import { SectionHeader } from "../../components/section-header";
import { SpecTable, type SpecRow } from "../../components/spec-table";
import { StatusDot } from "../../components/status-dot";
import { apiStore, getAdapter, isApiError } from "../../lib/api";
import { useApiMeta, useConnection, useDevice, useDevices, useServices, useStatus } from "../../lib/api/hooks";
import type { DeviceRecord } from "../../lib/api/types";
import {
  agoText,
  aliasFromDraft,
  confidenceLabel,
  deviceAgentAction,
  deviceDot,
  devicePrompt,
  displayName,
  drivenBy,
  firmwareText,
  forgetConfirm,
  identityRows,
  kindWord,
  presenceText,
  roleText,
  showsFirmware,
  stateLabel,
  transportWord,
} from "../../lib/devices";
import { Adv, useMode } from "../../lib/mode";
import { AgentButton, useAgentReachable } from "./agent-button";

const BACK = (
  <ButtonLink href={hrefTo("/devices")} size="sm">
    All devices
  </ButtonLink>
);

function errorText(err: unknown, fallback: string): string {
  if (!isApiError(err)) return fallback;
  if (err.code === "network") return "Could not reach the node. Check that it is running and try again.";
  return err.problem.detail || err.problem.title || fallback;
}

function when(iso: string, now: number): string {
  const t = Date.parse(iso);
  return Number.isFinite(t) ? `${new Date(t).toLocaleString()} (${agoText(iso, now)})` : "unknown";
}

/** A value with a copy button beside it. Wraps instead of overflowing on narrow screens. */
function Copyable({ text, label }: { text: string; label: string }) {
  return (
    <span className="inline-flex flex-wrap items-center justify-end gap-2">
      <span className="min-w-0 break-all">{text}</span>
      <CopyButton text={text} ariaLabel={`Copy ${label}`} />
    </span>
  );
}

function RenameForm({ device }: { device: DeviceRecord }) {
  const shown = displayName(device);
  const [draft, setDraft] = useState(shown);
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState<{ kind: "ok" | "error"; text: string } | null>(null);
  const synced = useRef(shown);

  // Follow a rename from another tab or the agent, unless the user is in the middle of typing.
  useEffect(() => {
    if (draft === synced.current) setDraft(shown);
    synced.current = shown;
  }, [shown]);

  const dirty = draft.trim() !== shown.trim();

  const save = async (e: FormEvent) => {
    e.preventDefault();
    if (!dirty || busy) return;
    setBusy(true);
    setMessage(null);
    try {
      const adapter = await getAdapter();
      await adapter.patchDevice(device.id, { alias: aliasFromDraft(draft, device) });
      await apiStore.refreshDevices();
      announce("renamed");
      setMessage({ kind: "ok", text: "Renamed." });
    } catch (err) {
      if (redirectIfUnauthenticated(err)) return;
      setMessage({ kind: "error", text: errorText(err, "Could not rename the device. Try again.") });
    } finally {
      setBusy(false);
    }
  };

  return (
    <form onSubmit={(e) => void save(e)} className="flex flex-col gap-2">
      <label htmlFor="device-name" className="font-mono text-[10px] uppercase tracking-widest text-muted-foreground">
        Name
      </label>
      <div className="flex flex-wrap items-stretch gap-2">
        <Input
          id="device-name"
          value={draft}
          maxLength={64}
          autoComplete="off"
          spellCheck={false}
          aria-describedby="device-name-help"
          onChange={(e) => {
            setDraft(e.target.value);
            setMessage(null);
          }}
          className="min-w-0 flex-1 basis-56"
        />
        <Button type="submit" disabled={!dirty} loading={busy}>
          Save name
        </Button>
      </div>
      <p id="device-name-help" className="font-mono text-[11px] leading-snug text-muted-foreground">
        Leave it empty to use the detected name: {device.name}
      </p>
      {message ? (
        <p role={message.kind === "error" ? "alert" : "status"} className={message.kind === "error" ? "font-mono text-xs text-destructive" : "font-mono text-xs text-muted-foreground"}>
          {message.text}
        </p>
      ) : null}
    </form>
  );
}

function ForgetButton({ device }: { device: DeviceRecord }) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const forget = async () => {
    setBusy(true);
    setError(null);
    try {
      const adapter = await getAdapter();
      await adapter.forgetDevice(device.id);
      announce("forgotten");
      // Leave first: the list may refresh to a registry without this device before the page is gone.
      navigate("/devices");
      void apiStore.refreshDevices();
    } catch (err) {
      if (redirectIfUnauthenticated(err)) return;
      setError(errorText(err, "Could not forget the device. Try again."));
      setBusy(false);
    }
  };

  return (
    <div className="flex flex-col gap-2">
      <div>
        <Button variant="destructive" confirm={forgetConfirm(device)} loading={busy} onClick={() => void forget()}>
          Forget
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

export default function DeviceDetail({ id }: { id: string }) {
  const device = useDevice(id);
  const devices = useDevices();
  const services = useServices();
  const status = useStatus();
  const { disconnected } = useConnection();
  const { ready } = useApiMeta();
  const { mode } = useMode();
  const reachable = useAgentReachable();
  const now = useNow(true, 15_000);

  if (devices === null) {
    return ready ? (
      <div className="flex flex-col gap-6">
        <div>{BACK}</div>
        <Empty kicker="unavailable" title="The device list could not be loaded">
          The node did not answer. The device appears here as soon as it does.
        </Empty>
      </div>
    ) : (
      <p role="status" className="font-mono text-xs text-muted-foreground">
        Loading device
      </p>
    );
  }

  if (device === null) {
    return (
      <div className="flex flex-col gap-6">
        <div>{BACK}</div>
        <Empty
          kicker="404"
          title="No such device"
          action={
            <ButtonLink href={hrefTo("/devices")} size="sm">
              Back to Devices
            </ButtonLink>
          }
        >
          This node has no device with that id. It may have been forgotten, or it is not plugged in any more.
        </Empty>
      </div>
    );
  }

  const can = (action: string) => device.actions.includes(action as DeviceRecord["actions"][number]);
  const name = displayName(device);
  const service = device.service_id ? services?.find((s) => s.id === device.service_id) : undefined;
  const action = deviceAgentAction(device);
  const rows: SpecRow[] = [];
  const add = (label: string, value: ReactNode, minMode?: "advanced") => rows.push({ label, value, minMode });

  add("Detected name", device.name);
  add("Confidence", `${confidenceLabel(device)}${device.match?.rule ? ` (${device.match.rule})` : ""}`);
  if (showsFirmware(device)) add("Firmware", firmwareText(device));
  add("Driven by", device.driver?.interface ? `${drivenBy(device)} · ${device.driver.interface}${device.driver.live ? "" : " (not answering)"}` : drivenBy(device));
  if (device.service_id) {
    add(
      "Service",
      <a href={hrefTo(`/services/${device.service_id}`)} className="underline underline-offset-2 hover:text-muted-foreground focus-visible:ring-2 focus-visible:ring-foreground focus-visible:outline-none">
        {service?.plain_name ?? device.service_id}
      </a>,
    );
  }
  add("Transport", transportWord(device.transport));
  const role = roleText(device.role);
  if (role) add("Role", role);
  for (const r of identityRows(device)) add(r.label, r.copy ? <Copyable text={r.value} label={r.label.toLowerCase()} /> : <span className="break-all">{r.value}</span>);
  add("Capabilities", device.capabilities.length > 0 ? device.capabilities.join(", ") : "none reported");
  if (device.kb_entry) {
    add(
      "Knowledge base",
      <ButtonLink href={hrefTo(`/kb/${device.kb_entry}`)} size="sm">
        Open the entry
      </ButtonLink>,
    );
  }
  add("First seen", when(device.first_seen, now));
  add("Last seen", when(device.last_seen, now));
  add("Reason code", device.reason_code, "advanced");

  const reserved = device.actions.filter((a) => a === "identify" || a === "pair" || a === "authorize");

  return (
    <div className="flex flex-col gap-6">
      <div>{BACK}</div>

      <SectionHeader kicker="device" title={name} level={1} />

      <div className="flex flex-wrap items-center gap-x-3 gap-y-2">
        <StatusDot state={deviceDot(device.state, disconnected)} />
        <Badge>{kindWord(device.kind)}</Badge>
        <Badge>{stateLabel(device.state, disconnected)}</Badge>
        {confidenceLabel(device) === "likely" ? <Badge>likely</Badge> : null}
        <span className="font-mono text-xs text-muted-foreground">{presenceText(device, now, disconnected)}</span>
      </div>

      <p className="max-w-prose font-mono text-sm leading-relaxed text-foreground">{device.reason}</p>

      {can("rename") ? (
        <RenameForm key={device.id} device={device} />
      ) : null}

      {can("agent") ? (
        <div className="flex flex-col gap-2">
          <AgentButton
            label={action.label}
            from={name.slice(0, 60)}
            goal={action.goal}
            reachable={reachable}
            prompt={() => devicePrompt({ device, services: services ?? [], status }).text}
          />
          <p className="max-w-prose font-mono text-[11px] leading-snug text-muted-foreground">The agent gives you steps or a proposal. It does not touch the hardware itself.</p>
        </div>
      ) : null}

      <SpecTable rows={rows} mode={mode} title="This device" />

      {reserved.length > 0 ? (
        <Panel title="more actions" tint>
          <div className="flex flex-wrap items-center gap-3">
            {reserved.map((a) => (
              <Button key={a} disabled size="sm">
                {a === "identify" ? "Identify" : a === "pair" ? "Pair" : "Authorize"}
              </Button>
            ))}
            <span className="font-mono text-[11px] text-muted-foreground">These arrive with Layer 6.</span>
          </div>
        </Panel>
      ) : null}

      <Adv>
        <JsonBlock value={device.detail} summary="Raw detection facts" />
      </Adv>

      {can("forget") ? (
        <div className="border-t border-border pt-4">
          <ForgetButton device={device} />
        </div>
      ) : null}
    </div>
  );
}
