// HAM confirmation per account and the per-channel uplink send switch (os/6-transport/README.md §5, §15.5). Neither is a bare switch:
// turning one on answers with the exact text and its hash, the person reads it, and confirming records that hash. Turning one off needs
// no confirmation.
import { useState } from "react";
import { announce } from "../../../app/LiveRegion";
import { Badge } from "../../../components/badge";
import { Toggle } from "../../../components/toggle";
import { apiStore, getAdapter } from "../../../lib/api";
import { useConversationsOf } from "../../../lib/api/hooks";
import type { BridgeSettings } from "../../../lib/api/types";
import { useConsentFlow } from "../../messages/net/consent";
import { convTitle, plain } from "../../messages/net/model";
import { problemText } from "../model";
import { Feedback, FEEDBACK, Row } from "../row";
import { GATES_HELP, HAM_HELP, UPLINK_HELP } from "./model";

function HamSwitch({ h }: { h: BridgeSettings["ham"][number] }) {
  const { dialog, attempt } = useConsentFlow();
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const press = async (next: boolean) => {
    setBusy(true);
    setError(null);
    try {
      const adapter = await getAdapter();
      const out = await attempt((hash) => adapter.bridge.patchSettings({ ham: [{ account: h.account, confirmed: next, ...(next && hash ? { accept_warning: hash } : {}) }] }));
      if (out.ok) {
        apiStore.adoptBridgeSettings(out.value);
        void apiStore.refreshNetworks();
        announce(next ? "Readable sends confirmed for this account." : "Readable sends are off for this account again.");
      } else if (!out.cancelled) setError(problemText(out.error));
    } finally {
      setBusy(false);
    }
  };
  return (
    <li className="flex flex-col gap-2 border border-border p-3" data-testid={`ham-${h.account}`}>
      <p className="flex flex-wrap items-center gap-2 font-mono text-sm text-foreground">
        <span className="min-w-0 break-words">{plain(h.account, 60)}</span>
        <Badge>{h.licensed ? "licensed" : "not licensed"}</Badge>
      </p>
      <div>
        <Toggle pressed={h.confirmed} onPressedChange={(next) => void press(next)} busy={busy}>
          readable sends confirmed <span aria-hidden="true">{h.confirmed ? "[ON]" : "[OFF]"}</span>
        </Toggle>
      </div>
      {dialog}
      <Feedback text={error} problem />
    </li>
  );
}

function ChannelSwitch({ c, title }: { c: BridgeSettings["channels"][number]; title: string }) {
  const { dialog, attempt } = useConsentFlow();
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const press = async (next: boolean) => {
    setBusy(true);
    setError(null);
    try {
      const adapter = await getAdapter();
      const out = await attempt((hash) => adapter.bridge.patchSettings({ channels: [{ conv: c.conv, send: next, ...(next && hash ? { accept_warning: hash } : {}) }] }));
      if (out.ok) {
        apiStore.adoptBridgeSettings(out.value);
        announce(next ? "Sending on this channel is on." : "Sending on this channel is off.");
      } else if (!out.cancelled) setError(problemText(out.error));
    } finally {
      setBusy(false);
    }
  };
  return (
    <li className="flex flex-col gap-2 border border-border p-3" data-testid={`channel-${c.conv}`}>
      <p className="flex flex-wrap items-center gap-2 font-mono text-sm text-foreground">
        <span dir="auto" className="min-w-0 break-words [unicode-bidi:isolate]">
          {title}
        </span>
        {c.uplink === true ? <Badge className="border-foreground text-foreground">published to the internet</Badge> : null}
      </p>
      <div>
        <Toggle pressed={c.send} onPressedChange={(next) => void press(next)} busy={busy}>
          sending <span aria-hidden="true">{c.send ? "[ON]" : "[OFF]"}</span>
        </Toggle>
      </div>
      {dialog}
      <Feedback text={error} problem />
    </li>
  );
}

export function GatesRows({ settings }: { settings: BridgeSettings | null }) {
  const nets = Array.from(new Set((settings?.channels ?? []).map((c) => c.network)));
  const convs = useConversationsOf(nets);
  const titleOf = (conv: string, net: string) => {
    const c = (convs[net] ?? []).find((x) => x.id === conv);
    return c ? convTitle(c) : conv;
  };
  return (
    <>
      <Row id="gates" label="Confirmations" help={GATES_HELP}>
        <p className={`text-muted-foreground ${FEEDBACK}`}>{settings === null ? "Not reported yet." : "Below: your licensed boards and your channels that upload."}</p>
      </Row>
      <Row id="ham" label="Licensed boards" help={HAM_HELP}>
        {settings && settings.ham.length > 0 ? (
          <ul className="flex flex-col gap-3">
            {settings.ham.map((h) => (
              <HamSwitch key={h.account} h={h} />
            ))}
          </ul>
        ) : (
          <p className={`text-muted-foreground ${FEEDBACK}`}>No board reports an amateur licence.</p>
        )}
      </Row>
      <Row id="uplink" label="Channel sending" help={UPLINK_HELP}>
        {settings && settings.channels.length > 0 ? (
          <ul className="flex flex-col gap-3">
            {settings.channels.map((c) => (
              <ChannelSwitch key={c.conv} c={c} title={titleOf(c.conv, c.network)} />
            ))}
          </ul>
        ) : (
          <p className={`text-muted-foreground ${FEEDBACK}`}>No channels yet. They appear as the board reports them.</p>
        )}
      </Row>
    </>
  );
}
