// Settings, Relay & presence, Presence (os/7-relay/README.md §2.4, §2.6): the interval per transport, shown and inert. A write is
// refused by the node (409 no_such_transport; settings_locked for the Reticulum announce, which Layer 6 owns), and the page shows
// the node's own words for it, unchanged.
import { useId, useState, type FormEvent } from "react";
import { Button } from "../../../components/button";
import { Input } from "../../../components/input";
import { Panel } from "../../../components/panel";
import type { RelayStatus } from "../../../lib/api/types";
import { FEEDBACK, Row } from "../row";
import { AIRTIME_TEXT, heartbeatRows, HEARTBEAT_BANS, PATCH, PRESENCE_HELP, problemDetail, type HeartbeatRow } from "./model";
import { useRelayCall } from "./actions";
import { CallOutput } from "./call-view";
import { PeerText } from "../../services/shared";

/** The node refuses a heartbeat write (409 `no_such_transport`, `settings_locked` for rns); what the person reads is its own `detail`. */
export const HEARTBEAT_OPTS = { errorText: problemDetail };

export function HeartbeatLine({ row }: { row: HeartbeatRow }) {
  const id = useId();
  const [value, setValue] = useState(String(row.minutes));
  const [invalid, setInvalid] = useState<string | null>(null);
  const call = useRelayCall();
  const submit = async (e: FormEvent) => {
    e.preventDefault();
    if (call.state.pending) return;
    const text = value.trim();
    if (!/^\d{1,4}$/.test(text)) {
      setInvalid("Type a whole number of minutes; 0 means events only.");
      return;
    }
    setInvalid(null);
    // The write is refused in this version; the node's answer is what the person reads.
    await call.patch(PATCH.heartbeat(row.transport, Number(text)), HEARTBEAT_OPTS);
  };
  return (
    <li className="flex flex-col gap-2 border-b border-border py-3 first:pt-0 last:border-b-0 last:pb-0" data-testid={`heartbeat-${row.transport}`}>
      <form className="flex flex-wrap items-end gap-3" onSubmit={(e) => void submit(e)} noValidate>
        <div className="min-w-0 flex-1 basis-40">
          <p className="font-mono text-sm text-foreground">
            <PeerText>{row.transport}</PeerText>
          </p>
          <p className={`text-muted-foreground ${FEEDBACK}`} data-testid={`heartbeat-${row.transport}-interval`}>
            {row.interval}
            {row.elsewhere ? " (the Reticulum announce interval, set in Layer 6)" : ""}
          </p>
        </div>
        <div className="flex flex-col gap-1">
          <label htmlFor={`${id}-min`} className="font-mono text-[10px] uppercase tracking-widest text-muted-foreground">
            minutes
          </label>
          <Input id={`${id}-min`} className="w-24" inputMode="numeric" autoComplete="off" value={value} onChange={(e) => setValue(e.target.value)} />
        </div>
        <Button size="sm" type="submit" loading={call.state.pending} data-testid={`heartbeat-${row.transport}-set`}>
          Set
        </Button>
      </form>
      <CallOutput call={call} problem={invalid} />
    </li>
  );
}

export function PresenceSection({ relay }: { relay: RelayStatus }) {
  const rows = heartbeatRows(relay);
  return (
    <Panel title="presence" suffix="heartbeat" flush>
      <Row id="heartbeat" label="Heartbeat" help={PRESENCE_HELP}>
        {rows.length === 0 ? (
          <p className={`text-muted-foreground ${FEEDBACK}`}>The node lists no transports.</p>
        ) : (
          <ul className="flex flex-col" data-testid="heartbeat-list">
            {rows.map((r) => (
              <HeartbeatLine key={r.transport} row={r} />
            ))}
          </ul>
        )}
      </Row>
      <Row id="airtime" label="Why events only">
        <p className={`max-w-prose text-foreground ${FEEDBACK}`} data-testid="airtime-line">
          {AIRTIME_TEXT}
        </p>
      </Row>
      <Row id="bans" label="Bans" help="These are checked before any beat is allowed, once a transport can take one.">
        <ul className={`list-disc ps-5 text-foreground ${FEEDBACK}`}>
          {HEARTBEAT_BANS.map((b) => (
            <li key={b}>{b}</li>
          ))}
        </ul>
      </Row>
    </Panel>
  );
}
