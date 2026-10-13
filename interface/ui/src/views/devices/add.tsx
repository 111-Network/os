// Add device (spec 6.3): one card per class of thing that can be attached. Honest copy only: where a
// class is not built yet, the card says so and offers nothing that would not work.
import { useId, useState, type ReactNode } from "react";
import { hrefTo } from "../../app/location";
import { Badge } from "../../components/badge";
import { Button, ButtonLink } from "../../components/button";
import { CopyButton } from "../../components/copy-button";
import { Panel } from "../../components/panel";
import { SectionHeader } from "../../components/section-header";
import { useDevices, useServices, useStatus } from "../../lib/api/hooks";
import { PAIR_COMMAND, alsoLabel, isMdnsUrl, pairingHelp, registryPrompt, type PairingHelp } from "../../lib/devices";
import { AgentButton, useAgentReachable } from "./agent-button";

const P = "max-w-prose font-mono text-xs leading-relaxed text-muted-foreground";

function Code({ children }: { children: ReactNode }) {
  return <code className="border border-border bg-secondary/40 px-1.5 py-0.5 font-mono text-xs text-foreground">{children}</code>;
}

function Role({ title, badge, children }: { title: string; badge?: string; children: ReactNode }) {
  return (
    <section className="flex flex-col gap-2 py-4 first:pt-0 last:pb-0">
      <div className="flex flex-wrap items-center gap-2">
        <h3 className="font-mono text-sm text-foreground">{title}</h3>
        {badge ? <Badge>{badge}</Badge> : null}
      </div>
      {children}
    </section>
  );
}

function ControlThisNode({ help }: { help: PairingHelp }) {
  const [showPair, setShowPair] = useState(false);
  const panelId = useId();
  return (
    <div className="flex flex-col gap-3">
      <p className={P}>{help.body}</p>
      {help.primaryUrl ? (
        <ul className="flex flex-col gap-2">
          <li className="flex flex-wrap items-center gap-2 font-mono text-xs text-foreground">
            <span className="min-w-0 break-all">{help.primaryUrl}</span>
            <CopyButton text={help.primaryUrl} ariaLabel={`Copy ${help.primaryUrl}`} />
          </li>
          {help.alsoUrls.map((u) => (
            <li key={u} className="flex flex-wrap items-center gap-2 font-mono text-xs text-muted-foreground">
              <span className="min-w-0 break-all">{alsoLabel(u)}</span>
              <CopyButton text={u} ariaLabel={`Copy ${u}`} />
            </li>
          ))}
        </ul>
      ) : null}
      {help.primaryUrl && isMdnsUrl(help.primaryUrl) ? (
        <p className={P}>This node only has a .local name right now. Android browsers cannot open those. Connect the node to Wi-Fi or Ethernet to get a numeric address.</p>
      ) : null}
      {help.command ? (
        <p className={P}>
          On the node run <Code>{help.command}</Code> for the pair code.
        </p>
      ) : null}
      {help.settingsHref ? (
        <div>
          <ButtonLink href={hrefTo(help.settingsHref)} size="sm">
            Open Settings
          </ButtonLink>
        </div>
      ) : null}
      {help.pairUrl ? (
        <div className="flex flex-col gap-3">
          <div>
            <Button size="sm" aria-expanded={showPair} aria-controls={panelId} onClick={() => setShowPair((v) => !v)}>
              {showPair ? "Hide pair address" : "Show pair address"}
            </Button>
          </div>
          <div id={panelId} hidden={!showPair} className="flex flex-col gap-2 border border-border p-3">
            {showPair ? (
              <>
                <p className="font-pixel text-lg leading-snug break-all text-foreground md:text-2xl">{help.pairUrl}</p>
                <p className={P}>
                  Type this address on the phone, then enter the pair code from the node. Run <Code>{PAIR_COMMAND}</Code> on the node for a scannable code.
                </p>
                <CopyButton text={help.pairUrl} label="copy address" ariaLabel="Copy the pair address" />
              </>
            ) : null}
          </div>
        </div>
      ) : null}
    </div>
  );
}

export default function AddDevice() {
  const status = useStatus();
  const devices = useDevices();
  const services = useServices();
  const reachable = useAgentReachable();
  const help = pairingHelp(status);

  const guide = (hint: string) => () => registryPrompt({ devices: devices ?? [], services: services ?? [], status, hint });

  return (
    <div className="flex flex-col gap-6">
      <SectionHeader
        kicker="devices"
        title="Add device"
        level={1}
        end={
          <ButtonLink href={hrefTo("/devices")} size="sm">
            All devices
          </ButtonLink>
        }
      />
      <p className="max-w-prose font-mono text-xs leading-relaxed text-muted-foreground">
        Pick what you want to connect. Hardware shows up in the list by itself once the node can see it.
      </p>

      <div className="grid gap-4 md:grid-cols-2">
        <Panel title="LoRa board over USB" className="flex flex-col" bodyClassName="flex flex-1 flex-col gap-4">
          <p className={P}>
            Plug it in. It appears in the list by itself within a few seconds. The board shows as &lsquo;likely&rsquo; until it is identified.
          </p>
          <AgentButton
            label="Guide me with the agent"
            from="Add device: LoRa board"
            goal="connect"
            variant="default"
            reachable={reachable}
            prompt={guide("I am connecting a LoRa board over USB")}
          />
        </Panel>

        <Panel title="Bluetooth device" className="flex flex-col" bodyClassName="flex flex-1 flex-col gap-4">
          <p className={P}>Bluetooth pairing arrives with Layer 6. Adapters already show in the list.</p>
          <AgentButton
            label="Guide me with the agent"
            from="Add device: Bluetooth"
            goal="connect"
            variant="default"
            reachable={reachable}
            prompt={guide("I want to connect a Bluetooth device")}
          />
        </Panel>

        <Panel title="Phone or laptop" className="md:col-span-2" bodyClassName="divide-y divide-border">
          <Role title="Control this node">
            <ControlThisNode help={help} />
          </Role>
          <Role title="Make it a node of its own">
            <p className={P}>Run the install command on a computer with the phone connected over USB-C. See the install docs.</p>
            <AgentButton
              label="Ask the agent how"
              from="Add device: phone as a node"
              goal="connect"
              variant="default"
              reachable={reachable}
              prompt={guide("I want to make an old phone a node of its own, installed over USB-C")}
            />
          </Role>
          <Role title="Message through this node" badge="coming with Layer 6">
            <p className={P}>
              Sending and receiving mesh messages from a phone through this node needs a mesh client on the phone. Nothing to set up yet.
            </p>
          </Role>
        </Panel>

        <Panel title="Handheld radio (Baofeng-class)" className="md:col-span-2" bodyClassName="flex flex-col gap-3">
          <div>
            <Badge>coming</Badge>
          </div>
          <p className={P}>
            Licensed operators only. Amateur bands allow plaintext only — encrypted mesh traffic cannot go over them. We will detect and explain first; an
            experimental tier comes later.
          </p>
        </Panel>
      </div>
    </div>
  );
}
