// Dev gallery: every primitive in every state, for visual review. U2 mounts it at /_gallery in dev only.
import { useEffect, useState, type ReactNode } from "react";
import { AttentionBadge } from "../components/attention-badge";
import { Badge } from "../components/badge";
import { Button, ButtonLink } from "../components/button";
import { CopyButton } from "../components/copy-button";
import { DataTable, type DataColumn } from "../components/data-table";
import { DiffView } from "../components/diff-view";
import { Disclosure } from "../components/disclosure";
import { Empty } from "../components/empty";
import { Input } from "../components/input";
import { JsonBlock } from "../components/json-block";
import { MiniTerminal } from "../components/mini-terminal";
import { Panel } from "../components/panel";
import { SectionHeader } from "../components/section-header";
import { Segmented } from "../components/segmented";
import { SpecTable, type SpecRow, type UiMode } from "../components/spec-table";
import { Sparkline } from "../components/sparkline";
import { StatusDot, type DotState } from "../components/status-dot";
import { Textarea } from "../components/textarea";
import { TerminalChrome } from "../components/terminal-chrome";
import { Toggle } from "../components/toggle";

const DOT_STATES: DotState[] = ["live", "unset", "error", "starting", "unknown"];

const SPEC_ROWS: SpecRow[] = [
  { label: "node", value: "111-7f3a (phone)" },
  { label: "version", value: "0.4.2" },
  { label: "uptime", value: "3d 4h" },
  { label: "bech32 id", value: "node1qpz...x9k2", minMode: "advanced" },
  { label: "last probe", value: "2 s ago", minMode: "advanced" },
];

interface PeerRow {
  id: string;
  name: string;
  hops: number;
  seen: string;
  note: string;
}

const PEERS: PeerRow[] = [
  { id: "a", name: "kitchen-relay", hops: 1, seen: "2 s ago", note: "LoRa, SNR 7.5 dB" },
  { id: "b", name: "roof-hub", hops: 2, seen: "14 s ago", note: "" },
  { id: "c", name: "far-node", hops: 4, seen: "3 min ago", note: "TCP via gateway" },
];

const PEER_COLUMNS: DataColumn<PeerRow>[] = [
  { key: "name", header: "Node", cell: (r) => r.name },
  { key: "hops", header: "Hops", cell: (r) => r.hops, align: "end" },
  { key: "seen", header: "Last heard", cell: (r) => r.seen, muted: true, align: "end" },
];

const LOG_LINES = [
  "$ journalctl --namespace=111node -n 6",
  "rnsd: interface TCPServer up",
  "rnsd: announce sent",
  "111d: services recomputed (7)",
  "111d: events ring 41/256",
  "agent: model health 200",
];

const SAMPLE_DIFF = `diff --git a/etc/111/node.toml b/etc/111/node.toml
--- a/etc/111/node.toml
+++ b/etc/111/node.toml
@@ -3,4 +3,5 @@
 [dashboard]
-lan = false
+lan = true
+port = 11180
 [agent]`;

const SAMPLE_JSON = { node: { label: "111-7f3a", category: "phone" }, services: [{ id: "agent", state: "live" }], ok: true };

function Block({ name, note, children }: { name: string; note?: string; children: ReactNode }) {
  return (
    <section aria-label={name} className="flex flex-col gap-4">
      <SectionHeader kicker={note ?? "primitive"} title={name} />
      {children}
    </section>
  );
}

function Row({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="flex flex-col gap-2">
      <span className="font-mono text-[10px] uppercase tracking-widest text-muted-foreground">{label}</span>
      <div className="flex flex-wrap items-center gap-3">{children}</div>
    </div>
  );
}

/** Dev only: make both clipboard paths fail so the "select to copy" state can be reviewed. */
function useBreakClipboard(on: boolean) {
  useEffect(() => {
    if (!on) return;
    const exec = document.execCommand;
    // `navigator.clipboard` is a prototype getter, so an own property shadows it and deleting restores it.
    Object.defineProperty(navigator, "clipboard", { value: undefined, configurable: true });
    document.execCommand = () => false;
    return () => {
      delete (navigator as unknown as Record<string, unknown>)["clipboard"];
      document.execCommand = exec;
    };
  }, [on]);
}

export default function Gallery() {
  const [mode, setMode] = useState<UiMode>("simple");
  const [pressed, setPressed] = useState(true);
  const [lines, setLines] = useState<string[]>(LOG_LINES);
  const [breakClipboard, setBreakClipboard] = useState(false);
  const [confirmed, setConfirmed] = useState("none yet");
  const [loading, setLoading] = useState(false);
  const [filter, setFilter] = useState("a");
  useBreakClipboard(breakClipboard);

  return (
    <main className="mx-auto flex max-w-4xl flex-col gap-12 px-4 py-8 lg:px-8">
      <SectionHeader level={1} kicker="dev only" title="Primitive gallery" end={<Badge>/_gallery</Badge>} />

      <Block name="Tokens" note="design system">
        <div className="grid grid-cols-2 gap-2 sm:grid-cols-4">
          {[
            ["background", "bg-background"],
            ["card", "bg-card"],
            ["secondary", "bg-secondary"],
            ["border", "bg-border"],
            ["muted-foreground", "bg-muted-foreground"],
            ["foreground", "bg-foreground"],
            ["destructive", "bg-destructive"],
          ].map(([name, cls]) => (
            <div key={name} className="flex items-center gap-2 border border-border p-2">
              <span aria-hidden="true" className={`h-4 w-4 shrink-0 border border-border ${cls}`} />
              <span className="font-mono text-[10px] text-muted-foreground">{name}</span>
            </div>
          ))}
        </div>
        <div className="font-pixel-line text-3xl font-bold">font-pixel-line 111 NODE</div>
        <div className="font-pixel text-lg">font-pixel 111.network</div>
        <div className="font-pixel text-lg font-bold">font-pixel bold 111.network</div>
        <div className="font-mono text-sm">font-mono the quick brown fox 0123456789</div>
      </Block>

      <Block name="TerminalChrome">
        <div className="border border-border">
          <TerminalChrome title="node" />
        </div>
        <div className="border border-border">
          <TerminalChrome title="agent" suffix="chat" end="ready" />
        </div>
      </Block>

      <Block name="Panel">
        <div className="grid gap-4 md:grid-cols-2">
          <Panel>Plain panel, no chrome.</Panel>
          <Panel title="services">With chrome title.</Panel>
          <Panel title="tint" tint>
            Tinted body (terminal tint).
          </Panel>
          <Panel title="elevated" elevated>
            Elevated: the single layered shadow.
          </Panel>
          <Panel title="flush" flush>
            <div className="px-4 py-2 text-xs text-muted-foreground">Flush: no body padding.</div>
          </Panel>
        </div>
      </Block>

      <Block name="SpecTable" note="rows filter by mode">
        <Segmented
          label="Gallery mode"
          value={mode}
          onChange={setMode}
          options={[
            { value: "simple", label: "SIMPLE" },
            { value: "advanced", label: "ADVANCED" },
          ]}
        />
        <SpecTable rows={SPEC_ROWS} mode={mode} title="Node" />
        <Panel title="bare inside flush panel" flush>
          <SpecTable rows={SPEC_ROWS.slice(0, 2)} bare />
        </Panel>
      </Block>

      <Block name="DataTable">
        <DataTable
          caption="Nearby nodes"
          columns={PEER_COLUMNS}
          rows={PEERS}
          rowKey={(r) => r.id}
          expand={(r) => (r.note ? <span className="text-xs">{r.note}</span> : null)}
        />
        <DataTable caption="Nearby nodes (no expand)" columns={PEER_COLUMNS} rows={PEERS.slice(0, 2)} rowKey={(r) => r.id} />
        <DataTable caption="Nearby nodes (empty)" columns={PEER_COLUMNS} rows={[]} rowKey={(r) => r.id} empty="No nearby nodes heard." />
      </Block>

      <Block name="Toggle">
        <Row label="on / off / disabled">
          <Toggle pressed={pressed} onPressedChange={setPressed}>
            LAN sharing: {pressed ? "ON" : "OFF"}
          </Toggle>
          <Toggle pressed={false} onPressedChange={() => {}}>
            Off
          </Toggle>
          <Toggle pressed onPressedChange={() => {}}>
            On
          </Toggle>
          <Toggle pressed={false} onPressedChange={() => {}} disabled>
            Disabled
          </Toggle>
        </Row>
      </Block>

      <Block name="Segmented">
        <Row label="SIMPLE | ADVANCED (arrow keys move and select)">
          <Segmented
            label="Interface mode"
            value={mode}
            onChange={setMode}
            options={[
              { value: "simple", label: "SIMPLE" },
              { value: "advanced", label: "ADVANCED" },
            ]}
          />
        </Row>
        <Row label="three options, middle disabled">
          <Segmented
            label="Transport filter"
            value={filter}
            onChange={setFilter}
            options={[
              { value: "a", label: "ALL" },
              { value: "b", label: "LORA", disabled: true },
              { value: "c", label: "TCP" },
            ]}
          />
        </Row>
      </Block>

      <Block name="Badge">
        <Row label="default">
          <Badge>phone</Badge>
          <Badge>v0.4.2</Badge>
          <Badge>layer 4</Badge>
        </Row>
      </Block>

      <Block name="StatusDot" note="the one color exception">
        <Row label="with label">
          {DOT_STATES.map((s) => (
            <StatusDot key={s} state={s} />
          ))}
        </Row>
        <Row label="label hidden (screen-reader text only)">
          {DOT_STATES.map((s) => (
            <StatusDot key={s} state={s} label={false} name="Mesh transport" />
          ))}
        </Row>
      </Block>

      <Block name="Button">
        <Row label="default / primary / destructive">
          <Button>Check for updates</Button>
          <Button variant="primary">Set up with the agent</Button>
          <Button variant="destructive">Stop service</Button>
        </Row>
        <Row label="small">
          <Button size="sm">Default</Button>
          <Button size="sm" variant="primary" arrow={false}>
            Primary
          </Button>
          <Button size="sm" variant="destructive">
            Destructive
          </Button>
        </Row>
        <Row label="disabled">
          <Button disabled>Default</Button>
          <Button variant="primary" disabled>
            Primary
          </Button>
          <Button variant="destructive" disabled>
            Destructive
          </Button>
        </Row>
        <Row label="loading (click toggles)">
          <Button loading={loading} onClick={() => setLoading(true)}>
            {loading ? "Checking" : "Check now"}
          </Button>
          {loading ? (
            <Button size="sm" onClick={() => setLoading(false)}>
              reset
            </Button>
          ) : null}
        </Row>
        <Row label="inline 2-step confirm (no modal). Last confirmed:">
          <Button variant="destructive" confirm="Restart node 111-7f3a" onClick={() => setConfirmed("restart")}>
            Restart node
          </Button>
          <Button confirm="Dismiss proposal" onClick={() => setConfirmed("dismiss")}>
            Dismiss
          </Button>
          <Button variant="primary" confirm="Accept proposal" onClick={() => setConfirmed("accept")}>
            Accept
          </Button>
          <span className="font-mono text-xs text-muted-foreground">{confirmed}</span>
        </Row>
        <Row label="ButtonLink (hash route, external)">
          <ButtonLink href="#/services/agent">Open service</ButtonLink>
          <ButtonLink href="#/" variant="primary">
            Go to agent
          </ButtonLink>
          <ButtonLink href="https://example.invalid/client" external>
            Open full client
          </ButtonLink>
        </Row>
      </Block>

      <Block name="Input / Textarea">
        <div className="grid gap-4 md:grid-cols-2">
          <label className="flex flex-col gap-1">
            <span className="font-mono text-[10px] uppercase tracking-widest text-muted-foreground">Node name</span>
            <Input placeholder="e.g. kitchen" />
          </label>
          <label className="flex flex-col gap-1">
            <span className="font-mono text-[10px] uppercase tracking-widest text-muted-foreground">Pair code</span>
            <Input defaultValue="ab12-" aria-invalid="true" />
          </label>
          <label className="flex flex-col gap-1">
            <span className="font-mono text-[10px] uppercase tracking-widest text-muted-foreground">Disabled</span>
            <Input defaultValue="available on the node itself" disabled />
          </label>
          <label className="flex flex-col gap-1">
            <span className="font-mono text-[10px] uppercase tracking-widest text-muted-foreground">Message</span>
            <Textarea placeholder="Ask the agent..." />
          </label>
        </div>
      </Block>

      <Block name="Disclosure">
        <Disclosure summary="Closed by default">
          <p className="text-xs text-muted-foreground">Native details element. Keyboard and screen readers come free.</p>
        </Disclosure>
        <Disclosure summary="Open by default" defaultOpen>
          <p className="text-xs text-muted-foreground">Open at first render.</p>
          <Disclosure summary="Nested, closed">
            <p className="text-xs text-muted-foreground">The marker follows each element's own state.</p>
          </Disclosure>
        </Disclosure>
      </Block>

      <Block name="MiniTerminal">
        <MiniTerminal title="rnsd" lines={LOG_LINES} />
        <MiniTerminal title="follow + live" suffix="tail" lines={lines} follow live maxHeightClass="max-h-40" />
        <Row label="feed">
          <Button size="sm" onClick={() => setLines((l) => [...l, `111d: tick ${l.length}`])}>
            append line
          </Button>
          <Button size="sm" onClick={() => setLines(LOG_LINES)}>
            reset
          </Button>
        </Row>
        <MiniTerminal title="empty" lines={[]} emptyText="No log lines yet." />
      </Block>

      <Block name="Sparkline">
        <Row label="series">
          <Sparkline values={[1, 3, 2, 5, 8, 6, 4, 7, 2, 1]} label="Messages per minute, last 10 minutes: peak 8" />
        </Row>
        <Row label="fixed range 0..100">
          <Sparkline values={[10, 40, 70, 100, 55]} min={0} max={100} label="Disk use 10 to 100 percent" />
        </Row>
        <Row label="flat / single / empty / has NaN">
          <Sparkline values={[4, 4, 4, 4]} label="Flat series" />
          <Sparkline values={[7]} label="One sample" />
          <Sparkline values={[]} label="No samples yet" />
          <Sparkline values={[1, Number.NaN, 5]} label="Series with a gap" />
        </Row>
      </Block>

      <Block name="DiffView">
        <DiffView diff={SAMPLE_DIFF} label="Proposed change to node.toml" />
      </Block>

      <Block name="JsonBlock">
        <JsonBlock value={SAMPLE_JSON} summary="status JSON" />
        <JsonBlock value={SAMPLE_JSON} summary="status JSON (open)" defaultOpen />
      </Block>

      <Block name="CopyButton">
        <Row label="copies, shows 'copied' for 1.5 s">
          <CopyButton text="node1qpz8k2x9k2" ariaLabel="Copy node id" />
          <CopyButton text="sudo -u node111 111 identity backup" label="copy command" />
        </Row>
        <Row label="simulate clipboard failure (both paths) to review the 'select to copy' state">
          <Toggle pressed={breakClipboard} onPressedChange={setBreakClipboard}>
            Break clipboard: {breakClipboard ? "ON" : "OFF"}
          </Toggle>
        </Row>
      </Block>

      <Block name="Empty">
        <Empty title="No nearby nodes heard.">Your node is running but alone on the mesh. Peers appear here as they announce themselves.</Empty>
        <Empty
          kicker="degraded"
          title="Local model is not answering."
          action={<Button variant="primary">Fix with the agent</Button>}
        >
          The health check has failed for 2 minutes. The agent can still read the knowledge base.
        </Empty>
      </Block>

      <Block name="AttentionBadge">
        <Row label="0 renders nothing | 1 | 12 | 120">
          <span className="font-mono text-xs text-muted-foreground">[</span>
          <AttentionBadge count={0} />
          <span className="font-mono text-xs text-muted-foreground">]</span>
          <AttentionBadge count={1} />
          <AttentionBadge count={12} />
          <AttentionBadge count={120} />
        </Row>
      </Block>

      <Block name="SectionHeader">
        <SectionHeader kicker="Overview" title="Your node is running" />
        <SectionHeader kicker="Services" title="Local AI agent" end={<StatusDot state="live" />} />
      </Block>
    </main>
  );
}
