// The Panel tab of a LIVE service page (os/5-messaging/README.md section 13): an adopted project's own web UI in an iframe on
// its own origin. Our chrome is the title, a health dot, reload (a new one-time ticket), "Open full client" (a new tab with
// its own ticket) and "Ask the agent about this panel". Only a registry-declared panel is ever framed; the ticket URL goes
// into the frame's `src` and is never shown. Where the panel cannot be framed (not running, a secure page needed, a panel
// that only opens on the node itself, a page loaded before the panel was switched on) the tab says so and there is no frame.
import { useEffect, useMemo, useState, useSyncExternalStore, type ReactNode } from "react";
import { redirectIfUnauthenticated } from "../../app/AuthGate";
import { announce } from "../../app/LiveRegion";
import { Button } from "../../components/button";
import { CopyButton } from "../../components/copy-button";
import { Disclosure } from "../../components/disclosure";
import { Empty } from "../../components/empty";
import { SpecTable } from "../../components/spec-table";
import { StatusDot } from "../../components/status-dot";
import { getAdapter } from "../../lib/api";
import { useStatus } from "../../lib/api/hooks";
import { panelsSeenAtLoad } from "../../lib/api/http";
import { panelOrigin } from "../../lib/api/panels";
import type { HostedPanel, ServiceRecord } from "../../lib/api/types";
import { Adv, useMode } from "../../lib/mode";
import { buildPrompt } from "../../lib/prompts";
import { AgentButton, useAgentReachable } from "../devices/agent-button";
import {
  allowAttr,
  cspListedAtLoad,
  currentEnv,
  healthDot,
  LOAD_GRACE_S,
  loopbackDashboardUrl,
  optionalHostsText,
  PANEL_CSP_TEXT,
  PANEL_EXPIRED_HELP,
  panelNodeAddress,
  PANEL_SANDBOX,
  panelGate,
  panelNotes,
  panelTitle,
  ticketSrc,
  UPSTREAM_DOWN_TEXT,
  watchFrameBlock,
  type PanelEnv,
} from "./panel-model";
import { openFullClient, PanelSessionController, type OpenedWindow, type SessionState } from "./panel-session";

const TEXT = "font-mono text-xs leading-relaxed text-muted-foreground";

// ---------------------------------------------------------------------------------------- notes

/** The honest copy for a panel: what it transmits, whose keys protect it, which outside hosts a browser may contact. */
export function PanelNotes({ id, optionalHosts = [], parts }: { id: string; optionalHosts?: readonly string[]; parts: ReadonlyArray<"transmits" | "keys" | "hosts" | "residual"> }) {
  const notes = panelNotes(id);
  const hosts = optionalHostsText(optionalHosts);
  const has = (p: "transmits" | "keys" | "hosts" | "residual") => parts.includes(p);
  if (!notes && !(hosts && has("hosts"))) return null;
  return (
    <div className="flex flex-col gap-2" data-testid="panel-notes">
      {notes && has("transmits") ? (
        <p className="max-w-prose font-mono text-xs leading-relaxed text-foreground">
          <span className="text-[10px] uppercase tracking-widest text-muted-foreground">this panel transmits </span>
          {notes.transmits}
        </p>
      ) : null}
      {notes && has("keys") ? <p className={`max-w-prose ${TEXT}`}>{notes.keys}</p> : null}
      {hosts && has("hosts") ? (
        <p className={`max-w-prose ${TEXT}`} data-testid="panel-hosts">
          {hosts}
        </p>
      ) : null}
      {notes && has("residual") ? (
        <Disclosure summary="If this node runs on a phone">
          <p className={`max-w-prose ${TEXT}`}>{notes.residual}</p>
        </Disclosure>
      ) : null}
    </div>
  );
}

// ---------------------------------------------------------------------------------------- chrome

function Chrome({ service, panel, actions }: { service: ServiceRecord; panel: HostedPanel; actions?: ReactNode }) {
  const status = useStatus();
  const reachable = useAgentReachable();
  const title = panelTitle(panel, service.plain_name);
  const dot = healthDot(panel.state.health);
  return (
    <div className="flex flex-col gap-3" data-testid="panel-chrome">
      <div className="flex flex-wrap items-center gap-x-4 gap-y-2">
        <StatusDot state={dot} label={false} name={`${title} panel`} />
        <span className="font-mono text-sm text-foreground">{title}</span>
        <span className="font-mono text-[10px] uppercase tracking-widest text-muted-foreground">
          {panel.state.health === "up" ? "service up" : panel.state.health === "down" ? "service down" : "service state unknown"}
        </span>
      </div>
      <div className="flex flex-wrap items-start gap-3">
        {actions}
        <AgentButton
          label="Ask the agent about this panel"
          from={`${title} panel`.slice(0, 60)}
          goal="panel.explain"
          variant="default"
          reachable={reachable}
          prompt={() => buildPrompt("panel.explain", { kind: "service", service }, { status })}
        />
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------------------- the frame

/** The iframe and what goes with it, for a session that is `ready`. Presentational: no network, no timers. */
export function PanelFrame({ src, title, panel, onLoad }: { src: string; title: string; panel: HostedPanel; onLoad?: () => void }) {
  return (
    <iframe
      title={`${title} panel`}
      src={src}
      sandbox={PANEL_SANDBOX}
      allow={allowAttr(panel.allow) ?? undefined}
      referrerPolicy="no-referrer"
      onLoad={onLoad}
      className="block h-[70vh] min-h-96 w-full border border-border bg-background"
    />
  );
}

function SessionStatus({ state, service, env, onRetry }: { state: SessionState; service: ServiceRecord; env: PanelEnv; onRetry: () => void }) {
  if (state.kind === "idle" || state.kind === "minting") {
    return (
      <p role="status" className={TEXT}>
        Opening the panel{"…"}
      </p>
    );
  }
  if (state.kind === "failed") {
    return (
      <div className="flex flex-col gap-3 border border-dashed border-border p-4" role="alert" data-testid="panel-failed">
        <p className="max-w-prose font-mono text-xs leading-relaxed text-foreground">{state.text}</p>
        {state.why === "loopback_only" ? <LoopbackUrl url={loopbackDashboardUrl(env, service.id)} /> : null}
        {state.why !== "loopback_only" ? (
          <div>
            <Button size="sm" onClick={onRetry}>
              Try again
            </Button>
          </div>
        ) : null}
      </div>
    );
  }
  return null;
}

function LoopbackUrl({ url }: { url: string }) {
  return (
    <div className="flex flex-col gap-2" data-testid="loopback-url">
      <p className={`max-w-prose ${TEXT}`}>On the node itself, open this address in a browser there:</p>
      <div className="flex flex-wrap items-center gap-3">
        <code className="min-w-0 break-all border border-border bg-secondary/20 px-2 py-1 font-mono text-xs text-foreground">{url}</code>
        <CopyButton text={url} label="copy address" ariaLabel="Copy the address of the dashboard on the node itself" />
      </div>
    </div>
  );
}

/**
 * What the panel's own page cannot say (spec 13, 14): the node address for the web client's connect dialog (the panel's own
 * host and port, with a copy button), and that a panel which reports an expired session is fixed by Reload. The page cannot
 * see inside the cross-origin frame, so it cannot re-mint by itself.
 */
export function PanelHelp({ panel, env }: { panel: Pick<HostedPanel, "port">; env: PanelEnv }) {
  const address = panelNodeAddress(panel, env);
  return (
    <div className="flex flex-col gap-2" data-testid="panel-help">
      <div className="flex flex-wrap items-center gap-3">
        <p className={`max-w-prose ${TEXT}`}>
          If the client asks for a node address, use{" "}
          <code className="break-all border border-border bg-secondary/20 px-1.5 py-0.5 text-foreground" data-testid="panel-node-address">
            {address}
          </code>
        </p>
        <CopyButton text={address} label="copy address" ariaLabel="Copy the node address for the panel's connect dialog" />
      </div>
      <p className={`max-w-prose ${TEXT}`}>{PANEL_EXPIRED_HELP}</p>
    </div>
  );
}

/**
 * Shown in place of the frame when this page's own security policy cannot allow it (the panel was switched on after the page
 * loaded). `observed`: the browser has just refused the frame; otherwise the page expects it to (the panel was not on when it
 * loaded) and has not tried. Reload is the only fix that keeps the frame inline: a document's policy never changes.
 */
export function PanelPolicyNotice({ observed, onReloadPage }: { observed: boolean; onReloadPage: () => void }) {
  return (
    <div className="flex flex-col gap-3 border border-dashed border-border p-4" role={observed ? "alert" : "status"} data-testid="panel-csp">
      <p className="max-w-prose font-mono text-xs leading-relaxed text-foreground">{PANEL_CSP_TEXT}</p>
      <div>
        <Button size="sm" variant="primary" onClick={onReloadPage}>
          Reload this page
        </Button>
      </div>
    </div>
  );
}

/** The framed panel: owns the session (mint on open, mint again on reload) and the "did it load" note. */
function Framed({ service, panel, env }: { service: ServiceRecord; panel: HostedPanel; env: PanelEnv }) {
  const id = service.id;
  const controller = useMemo(
    () =>
      new PanelSessionController({
        mint: async () => (await getAdapter()).panels.openSession(id),
        src: (url) => ticketSrc(url, panel, env),
        onError: (err) => void redirectIfUnauthenticated(err),
      }),
    // A new slot or host is a new panel origin: a new session. The record itself changing (health, dropped) is not.
    [id, panel.port, env.protocol, env.hostname, env.port],
  );
  const state = useSyncExternalStore(controller.subscribe, controller.getState, controller.getState);
  const [slow, setSlow] = useState(false);
  /** The generation of the frame whose `load` event has fired (a frame that loaded was handed a good ticket). */
  const [loadedGen, setLoadedGen] = useState(-1);
  const [note, setNote] = useState<string | null>(null);
  const title = panelTitle(panel, service.plain_name);

  // The page's own policy (section 13): `frame-src` was fixed when this document loaded and lists only the panels that were on
  // then. `expected` is the page's guess from its first `GET /panels`; `refused` is the browser's own word. Either way the
  // frame is not mounted (a blocked frame is the browser's blank error page and nothing else) and the tab says to reload.
  const origin = panelOrigin(panel, env);
  const expected = useMemo(() => cspListedAtLoad(panel.id, panelsSeenAtLoad()) === false, [panel.id]);
  const [refused, setRefused] = useState(false);
  const policyBlocked = expected || refused;
  useEffect(() => {
    if (typeof document === "undefined") return;
    return watchFrameBlock(document, origin, () => setRefused(true));
  }, [origin]);

  useEffect(() => {
    if (expected) return; // nothing to mint a ticket for: no frame will be mounted
    void controller.start();
    return () => controller.dispose();
  }, [controller, expected]);

  // A ticket is good for 30 s. A frame that has not loaded by then was handed a spent ticket: say so, and offer a new one.
  const gen = state.kind === "ready" && !policyBlocked ? state.gen : -1;
  const expiresIn = state.kind === "ready" ? state.expiresIn : 0;
  const loaded = gen >= 0 && loadedGen === gen;
  useEffect(() => {
    setSlow(false);
    if (gen < 0 || loaded) return;
    const h = window.setTimeout(() => setSlow(true), (expiresIn + LOAD_GRACE_S) * 1000);
    return () => window.clearTimeout(h);
  }, [gen, expiresIn, loaded]);

  const reload = () => {
    setNote(null);
    setSlow(false);
    announce("Opening the panel again with a new session.");
    void controller.reload();
  };
  const reloadPage = () => window.location.reload();
  const full = async () => {
    setNote(null);
    const r = await openFullClient(
      () => controller.fresh(),
      () => window.open("", "_blank") as OpenedWindow | null,
    );
    if (r.kind === "blocked") setNote("The browser blocked the new tab. Allow pop-ups for this page and try again.");
    else if (r.kind === "failed") setNote(r.text);
  };

  const usable = state.kind !== "failed" || state.why !== "loopback_only";
  return (
    <div className="flex flex-col gap-4">
      <Chrome
        service={service}
        panel={panel}
        actions={
          usable ? (
            <>
              {policyBlocked ? null : (
                <Button size="sm" onClick={reload} loading={state.kind === "minting"}>
                  Reload
                </Button>
              )}
              <Button size="sm" onClick={() => void full()}>
                Open full client
              </Button>
            </>
          ) : null
        }
      />
      {note ? (
        <p role="alert" className="max-w-prose font-mono text-xs leading-relaxed text-destructive">
          {note}
        </p>
      ) : null}
      {panel.secure_note ? <p className={`max-w-prose ${TEXT}`}>{panel.secure_note}</p> : null}
      <PanelNotes id={panel.id} parts={["transmits"]} />
      {panel.state.health === "down" ? (
        <p role="status" className="max-w-prose border border-dashed border-border p-3 font-mono text-xs leading-relaxed text-foreground" data-testid="panel-upstream-down">
          {UPSTREAM_DOWN_TEXT}
        </p>
      ) : null}
      {policyBlocked ? (
        <PanelPolicyNotice observed={refused} onReloadPage={reloadPage} />
      ) : (
        <>
          <SessionStatus state={state} service={service} env={env} onRetry={reload} />
          {state.kind === "ready" ? <PanelFrame key={state.gen} src={state.src} title={title} panel={panel} onLoad={() => setLoadedGen(state.gen)} /> : null}
          {slow && !loaded ? (
            <p role="status" className={`max-w-prose ${TEXT}`} data-testid="panel-slow">
              The panel has not finished loading. Its session ticket only lasts 30 seconds; press Reload to get a fresh one.
            </p>
          ) : null}
          {state.kind !== "failed" ? <PanelHelp panel={panel} env={env} /> : null}
        </>
      )}
      <PanelNotes id={panel.id} optionalHosts={panel.optional_hosts} parts={["hosts"]} />
      <PanelAdvanced panel={panel} />
    </div>
  );
}

// ------------------------------------------------------------------------------------------ advanced

/** Advanced: the listener slot and the shim's drop counter (`PanelState.dropped`, section 14). */
export function PanelAdvanced({ panel }: { panel: HostedPanel }) {
  const { mode } = useMode();
  const dropped = panel.state.dropped;
  return (
    <Adv>
      <section className="flex flex-col gap-3" data-testid="panel-advanced">
        <h2 className="font-mono text-[10px] uppercase tracking-widest text-muted-foreground">advanced</h2>
        <SpecTable
          mode={mode}
          rows={[
            { label: "Listener port", value: String(panel.port) },
            { label: "Entry path", value: panel.entry },
            { label: "Enabled", value: panel.state.enabled ? "yes" : "no" },
            { label: "Listening", value: panel.state.listening ? "yes" : "no" },
            { label: "Frames dropped", value: dropped === undefined ? "not reported" : String(dropped) },
            { label: "Needs a secure page", value: panel.needs_secure_context ? "yes" : "no" },
            { label: "Browser features", value: panel.allow.length > 0 ? panel.allow.join(", ") : "none" },
          ]}
        />
        {dropped !== undefined && dropped > 0 ? (
          <p className={`max-w-prose ${TEXT}`}>
            The panel read from the radio more slowly than it sent, so {dropped === 1 ? "1 frame was" : `${dropped} frames were`} dropped, oldest first. The radio itself lost nothing.
          </p>
        ) : null}
      </section>
    </Adv>
  );
}

// ------------------------------------------------------------------------------------------- the tab

/** The tab's body. `panel` is the live record (the `panels` frames keep it current). */
export function PanelTab({ service, panel }: { service: ServiceRecord; panel: HostedPanel }) {
  const [env] = useState(currentEnv);
  const gate = panelGate(panel, env, service.id);
  if (gate.kind === "frame") return <Framed service={service} panel={panel} env={env} />;
  return (
    <div className="flex flex-col gap-4">
      <Chrome service={service} panel={panel} />
      {panel.secure_note ? <p className={`max-w-prose ${TEXT}`}>{panel.secure_note}</p> : null}
      {gate.kind === "secure_context" ? (
        <Empty kicker="open it on the node" title="This panel needs a secure page" action={<LoopbackUrl url={gate.url} />}>
          {gate.text}
        </Empty>
      ) : (
        <Empty kicker="panel not running" title={gate.kind === "off" ? "Not switched on" : "Not listening yet"}>
          {gate.text}
        </Empty>
      )}
      <PanelAdvanced panel={panel} />
    </div>
  );
}
