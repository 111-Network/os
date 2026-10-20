// Region and callsign (D17 #10), the rnsd posture (D17 #5; §15.8 routing warning) and the phone-relay join (§11), on Settings, Transport.
import { useState, type FormEvent } from "react";
import { announce } from "../../../app/LiveRegion";
import { Badge } from "../../../components/badge";
import { Button } from "../../../components/button";
import { CopyButton } from "../../../components/copy-button";
import { Input } from "../../../components/input";
import { Toggle } from "../../../components/toggle";
import { apiStore } from "../../../lib/api";
import type { TransportJoin, TransportPosturePatch, TransportStatus } from "../../../lib/api/types";
import { INTERFACES_STATE_TEXT, ROUTING_WARNING, REGION_UNSET_TEXT, interfaceText, postureOutcomeText, regionText, routesOthers } from "../../../lib/api/transport";
import { useAction } from "../essentials";
import { Feedback, FEEDBACK, Row } from "../row";
import { CALLSIGN_HELP, IFAC_HELP, JOIN_HELP, JOIN_QR_NOTE, LAN_HELP, POSTURE_HELP, REGION_CONFIRM_TEXT, REGION_HELP, SHARE_INSTANCE_HELP, TRANSPORT_HELP } from "./model";

/** Run a posture patch, adopt the status the node answers with, and say whether it applied or went back. */
function usePosture() {
  const [action, run] = useAction();
  const [note, setNote] = useState<string | null>(null);
  const apply = async (patch: TransportPosturePatch) => {
    setNote(null);
    const result = await run((a) => a.transport.patchPosture(patch));
    if (!result) return false;
    apiStore.adoptTransport(result.status);
    const text = postureOutcomeText(result);
    setNote(text);
    announce(text);
    return !result.reverted;
  };
  return { action, note, apply };
}

export function RegionRows({ t }: { t: TransportStatus }) {
  const { action, note, apply } = usePosture();
  const [region, setRegion] = useState("");
  const [call, setCall] = useState(t.callsign ?? "");
  const wanted = region.trim().toUpperCase();
  const setNow = () => void apply({ region: wanted }).then((ok) => ok && setRegion(""));
  const saveCall = (e: FormEvent) => {
    e.preventDefault();
    void apply({ callsign: call.trim() === "" ? null : call.trim() });
  };
  return (
    <>
      <Row id="region" label="Radio region" help={REGION_HELP}>
        <div className="flex flex-col gap-3">
          <p className="font-mono text-sm text-foreground" data-testid="region-value">
            {regionText(t)}
          </p>
          {t.region === null ? <p className={`max-w-prose text-foreground ${FEEDBACK}`}>{REGION_UNSET_TEXT}</p> : null}
          <div className="flex flex-wrap items-end gap-2">
            <label className="flex flex-col gap-1">
              <span className="font-mono text-[10px] uppercase tracking-widest text-foreground">{t.region === null ? "pick your region" : "change the region"}</span>
              <Input value={region} autoComplete="off" spellCheck={false} placeholder="EU_868" className="w-44" onChange={(e) => setRegion(e.target.value)} />
            </label>
            <Button size="sm" confirm={REGION_CONFIRM_TEXT(wanted)} loading={action.pending} aria-disabled={wanted === "" || undefined} onClick={setNow} data-testid="region-set">
              set region
            </Button>
          </div>
          <Feedback text={action.error ?? note} problem={action.error !== null} />
        </div>
      </Row>
      <Row id="callsign" label="Callsign" help={CALLSIGN_HELP}>
        <form className="flex flex-wrap items-end gap-2" onSubmit={saveCall}>
          <label className="flex flex-col gap-1">
            <span className="font-mono text-[10px] uppercase tracking-widest text-foreground">callsign</span>
            <Input value={call} maxLength={16} autoComplete="off" spellCheck={false} placeholder="optional" className="w-44" onChange={(e) => setCall(e.target.value)} />
          </label>
          <Button type="submit" size="sm" loading={action.pending}>
            save
          </Button>
        </form>
      </Row>
    </>
  );
}

function PostureToggle({ id, label, help, on, onChange, busy }: { id: string; label: string; help: string; on: boolean; onChange: (next: boolean) => void; busy: boolean }) {
  return (
    <div className="flex flex-col gap-1" data-posture={id}>
      <Toggle pressed={on} onPressedChange={onChange} busy={busy} aria-describedby={`${id}-help`}>
        {label} <span aria-hidden="true">{on ? "[ON]" : "[OFF]"}</span>
      </Toggle>
      <p id={`${id}-help`} className={`max-w-prose text-muted-foreground ${FEEDBACK}`}>
        {help}
      </p>
    </div>
  );
}

export function PostureRows({ t }: { t: TransportStatus }) {
  const { action, note, apply } = usePosture();
  const p = t.posture;
  const warn = routesOthers(p);
  return (
    <>
      <Row id="posture" label="Reticulum posture" help={POSTURE_HELP}>
        <div className="flex flex-col gap-4">
          <PostureToggle id="share" label="Share instance" help={SHARE_INSTANCE_HELP} on={p.share_instance} busy={action.pending} onChange={(v) => void apply({ posture: { share_instance: v } })} />
          <PostureToggle id="transport" label="Route for others" help={TRANSPORT_HELP} on={p.enable_transport} busy={action.pending} onChange={(v) => void apply({ posture: { enable_transport: v } })} />
          <PostureToggle id="lan" label="LAN listener" help={LAN_HELP} on={p.lan.backbone_listen} busy={action.pending} onChange={(v) => void apply({ posture: { lan: { backbone_listen: v } } })} />
          <PostureToggle id="ifac" label="IFAC passphrase" help={IFAC_HELP} on={p.lan.ifac} busy={action.pending} onChange={(v) => void apply({ posture: { lan: { ifac: v } } })} />
          {warn ? (
            <p role="status" className={`max-w-prose border border-foreground p-3 text-foreground ${FEEDBACK}`} data-testid="routing-warning">
              {ROUTING_WARNING}
            </p>
          ) : null}
          <Feedback text={action.error ?? note} problem={action.error !== null} />
        </div>
      </Row>
      <Row id="interfaces" label="Interfaces" help={INTERFACES_STATE_TEXT[t.interfaces_state]}>
        {t.interfaces.length === 0 ? (
          <p className={`text-muted-foreground ${FEEDBACK}`}>No interface facts yet.</p>
        ) : (
          <ul className="border border-border" data-testid="interfaces">
            {t.interfaces.map((i) => (
              <li key={i.name} className="flex flex-wrap items-baseline justify-between gap-2 border-b border-border px-3 py-2 font-mono text-xs last:border-b-0">
                <span className="min-w-0 break-words text-foreground">{i.name}</span>
                <span className="text-muted-foreground">{interfaceText(i)}</span>
              </li>
            ))}
          </ul>
        )}
      </Row>
    </>
  );
}

export function JoinRow({ t }: { t: TransportStatus }) {
  const [action, run] = useAction();
  const [join, setJoin] = useState<TransportJoin | null>(null);
  const [reveal, setReveal] = useState(false);
  const show = async () => {
    const d = await run((a) => a.transport.join());
    if (d) {
      setJoin(d);
      setReveal(false);
    }
  };
  const off = !t.posture.lan.backbone_listen;
  return (
    <Row id="join" label="Join a phone" help={JOIN_HELP}>
      <div className="flex flex-col gap-3">
        <div className="flex flex-wrap items-center gap-2">
          <Button size="sm" loading={action.pending} aria-disabled={off || undefined} onClick={() => void show()} data-testid="join-show">
            show the join details
          </Button>
          {join ? (
            <Button
              size="sm"
              onClick={() => {
                setJoin(null);
                setReveal(false);
              }}
            >
              clear
            </Button>
          ) : null}
        </div>
        {off ? <p className={`text-muted-foreground ${FEEDBACK}`}>The LAN listener is off, so there is nothing to join. Turn it on above first.</p> : null}
        <Feedback text={action.error} problem />
        {join ? (
          <div className="flex flex-col gap-2 border border-border p-3 font-mono text-sm" data-testid="join-details">
            <p className="flex flex-wrap items-center gap-2 break-all text-foreground">
              {join.address}:{join.port}
              <CopyButton text={`${join.address}:${join.port}`} ariaLabel="Copy the hub's address and port" />
            </p>
            {join.ifac_name ? <p className="break-all text-muted-foreground">IFAC network name: {join.ifac_name}</p> : <p className="text-muted-foreground">No IFAC: anyone on the network can join.</p>}
            {join.ifac_passphrase ? (
              <p className="flex flex-wrap items-center gap-2 text-muted-foreground">
                IFAC passphrase:{" "}
                {reveal ? (
                  <>
                    <span className="break-all text-foreground" data-testid="join-passphrase">
                      {join.ifac_passphrase}
                    </span>
                    <CopyButton text={join.ifac_passphrase} ariaLabel="Copy the passphrase" />
                    <Button size="sm" onClick={() => setReveal(false)}>
                      hide
                    </Button>
                  </>
                ) : (
                  <>
                    <Badge>hidden</Badge>
                    <Button size="sm" onClick={() => setReveal(true)} data-testid="join-reveal">
                      show the passphrase
                    </Button>
                  </>
                )}
              </p>
            ) : null}
            <p className={`max-w-prose text-muted-foreground ${FEEDBACK}`}>{JOIN_QR_NOTE}</p>
          </div>
        ) : null}
      </div>
    </Row>
  );
}
