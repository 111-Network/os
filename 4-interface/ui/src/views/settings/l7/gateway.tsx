// Settings, Relay & presence, Gateway (os/7-relay/README.md §3, §6.2, §6.3, §12): the switch, the carrier (kind, server, mode, the two
// write-only secrets), the consent, "Check now", "Flush now" and the badge. Turning a carrier on is a consent round-trip: the node's
// exact words are shown and the call is repeated with their hash. A secret is typed into an empty field, sent once and never shown.
import { useId, useRef, useState, type FormEvent, type Ref } from "react";
import { Button } from "../../../components/button";
import { Input } from "../../../components/input";
import { Panel } from "../../../components/panel";
import { Segmented } from "../../../components/segmented";
import { Toggle } from "../../../components/toggle";
import type { RelayStatus } from "../../../lib/api/types";
import { cleanPeerText } from "../../../lib/api/messages";
import { METERED_CAP_TEXT } from "../../services/model";
import { relayBadge } from "../../services/fabric-model";
import { PeerText } from "../../services/shared";
import { FEEDBACK, Row } from "../row";
import { useRelayCall, type RelayCall } from "./actions";
import { CallOutput } from "./call-view";
import { EMPTY_SECRET, GATEWAY_HELP, HUB_ONLY_TEXT, isPlainHttp, MODE_HELP, PATCH, SECRET_HELP, secretState, TOKEN_CLEAR_NOTE, typeSecret, type SecretDraft } from "./model";

const LABEL = "font-mono text-[10px] uppercase tracking-widest text-muted-foreground";

/** What the carrier claim rests on, as text (§3.8): the carrier and its endpoint, both from the node and cleaned. */
function restsOn(g: RelayStatus["gateway"]): string | null {
  if (g.carrier === null) return null;
  const name = cleanPeerText(g.carrier).slice(0, 40);
  return g.endpoint ? `${name} at ${cleanPeerText(g.endpoint).slice(0, 80)}` : name;
}

/**
 * The write-only input. It is uncontrolled on purpose: no render ever puts what was typed into a `value` attribute, so a secret that is
 * typed and not yet sent is in the field's own state and nowhere in the markup. Never prefilled: the node never sends a secret.
 */
export function SecretInput({ id, name, isSet, inputRef, onText }: { id: string; name: string; isSet: boolean; inputRef?: Ref<HTMLInputElement>; onText?: (text: string) => void }) {
  return (
    <Input
      ref={inputRef}
      id={id}
      type="password"
      name={name}
      autoComplete="new-password"
      spellCheck={false}
      placeholder={isSet ? "set; type to replace" : "not set"}
      className="min-w-0 flex-1 basis-48"
      onInput={(e) => onText?.(e.currentTarget.value)}
    />
  );
}

function SecretField({ label, name, state, call, field }: { label: string; name: string; state: { set: boolean }; call: RelayCall; field: "token" | "topic_secret" }) {
  const id = useId();
  const input = useRef<HTMLInputElement>(null);
  const [draft, setDraft] = useState<SecretDraft>(EMPTY_SECRET);
  const send = async (next: string | null) => {
    const r = await call.patch(PATCH.secret(field, next), { done: next === null ? `${label} cleared.` : `${label} saved.` });
    if (r.ok && input.current) {
      input.current.value = "";
      setDraft(EMPTY_SECRET);
    }
  };
  const save = () => {
    const text = input.current?.value ?? "";
    if (text !== "" && !call.state.pending) void send(text);
  };
  return (
    <div className="flex flex-col gap-2" data-testid={`secret-${field}`}>
      <label htmlFor={`${id}-in`} className={LABEL}>
        {label} <span data-testid={`secret-${field}-state`}>({secretState(state)})</span>
      </label>
      <div className="flex flex-wrap items-center gap-2">
        <SecretInput id={`${id}-in`} name={name} isSet={state.set} inputRef={input} onText={(t) => setDraft((d) => typeSecret(d, t))} />
        <Button size="sm" aria-disabled={!draft.filled || call.state.pending || undefined} onClick={save}>
          Save
        </Button>
        {state.set ? (
          <Button size="sm" confirm={`Clear the ${label.toLowerCase()}`} onClick={() => void send(null)}>
            Clear
          </Button>
        ) : null}
      </div>
    </div>
  );
}

function CarrierForm({ relay, call, gated }: { relay: RelayStatus; call: RelayCall; gated: boolean }) {
  const c = relay.carrier;
  const id = useId();
  const [kind, setKind] = useState<"none" | "ntfy">(c.kind);
  const [server, setServer] = useState(c.server ?? "");
  const [mode, setMode] = useState<"5A" | "5B">(c.mode);
  const submit = async (e: FormEvent) => {
    e.preventDefault();
    if (call.state.pending || gated) return;
    await call.patch(PATCH.carrier(kind, server, mode), { done: kind === "none" ? "Carrier off." : "Carrier saved." });
  };
  const clear = kind === "ntfy" && isPlainHttp(server);
  return (
    <form className="flex flex-col gap-4" onSubmit={(e) => void submit(e)} noValidate>
      <Segmented<"none" | "ntfy">
        label="Carrier"
        value={kind}
        onChange={setKind}
        options={[
          { value: "none", label: "none" },
          { value: "ntfy", label: "ntfy" },
        ]}
      />
      {kind === "ntfy" ? (
        <>
          <div className="flex flex-col gap-1">
            <label htmlFor={`${id}-server`} className={LABEL}>
              ntfy server
            </label>
            <Input id={`${id}-server`} value={server} inputMode="url" autoComplete="off" spellCheck={false} placeholder="https://ntfy.example.org" onChange={(e) => setServer(e.target.value)} />
            {clear ? (
              <p role="note" className={`text-foreground ${FEEDBACK}`} data-testid="token-clear-note">
                {TOKEN_CLEAR_NOTE}
              </p>
            ) : null}
          </div>
          <div className="flex flex-col gap-2">
            <Segmented<"5A" | "5B"> label="Mode" value={mode} onChange={setMode} options={[{ value: "5A", label: "5A on events" }, { value: "5B", label: "5B held" }]} />
            <p className={`max-w-prose text-muted-foreground ${FEEDBACK}`}>{MODE_HELP[mode]}</p>
          </div>
        </>
      ) : null}
      <div>
        <Button type="submit" size="sm" variant="primary" arrow={false} loading={call.state.pending} aria-disabled={gated || undefined}>
          {kind === "none" ? "Save: no carrier" : "Save the carrier"}
        </Button>
      </div>
    </form>
  );
}

export function GatewaySection({ relay, hubGate }: { relay: RelayStatus; hubGate: boolean }) {
  const g = relay.gateway;
  const c = relay.carrier;
  const toggle = useRelayCall();
  const carrier = useRelayCall();
  const secrets = useRelayCall();
  const test = useRelayCall();
  const rests = restsOn(g);
  const configured = c.kind !== "none";
  const callNow = (which: "check" | "flush") =>
    void test.carrier(which, c, { done: which === "check" ? "Checking the carrier now. The result appears in the badge." : "Flushing now. Progress appears in the badge." });
  return (
    <Panel title="gateway" suffix="carrier" flush>
      <Row id="gateway-badge" label="Status" help={GATEWAY_HELP}>
        <div className="flex flex-col gap-3">
          <p className="font-mono text-sm text-foreground" data-testid="gateway-badge">
            {relayBadge(relay)}
          </p>
          {rests ? (
            <p className={`text-muted-foreground ${FEEDBACK}`} data-testid="gateway-rests-on">
              Resting on: <PeerText>{rests}</PeerText> ({g.mode ?? c.mode})
            </p>
          ) : null}
          {g.metered_cap_reached ? (
            <p role="status" className={`text-foreground ${FEEDBACK}`} data-testid="gateway-metered">
              {METERED_CAP_TEXT}
            </p>
          ) : null}
          <div className="flex flex-wrap items-center gap-2">
            <Button size="sm" aria-disabled={!configured || test.state.pending || undefined} loading={test.state.pending} onClick={() => configured && callNow("check")} data-testid="check-now">
              Check now
            </Button>
            <Button size="sm" aria-disabled={!configured || test.state.pending || undefined} onClick={() => configured && callNow("flush")} data-testid="flush-now">
              Flush now
            </Button>
          </div>
          {!configured ? <p className={`text-muted-foreground ${FEEDBACK}`}>Choose a carrier below first: there is nothing to check or flush yet.</p> : null}
          <CallOutput call={test} />
        </div>
      </Row>

      <Row id="gateway-on" label="Gateway" help={hubGate ? HUB_ONLY_TEXT : "Turns this hub's gateway role on. It acts only when a carrier is set below or a paired hub is reached over the internet."}>
        <Toggle
          pressed={g.enabled}
          disabled={hubGate}
          busy={toggle.state.pending}
          onPressedChange={(next) => void toggle.patch(PATCH.gatewayEnabled(next), { done: next ? "Gateway on." : "Gateway off." })}
        >
          <span>Gateway</span> <span aria-hidden="true">{g.enabled ? "[ON]" : "[OFF]"}</span>
        </Toggle>
        <CallOutput call={toggle} />
      </Row>

      <Row id="carrier" label="Carrier" help="A carrier moves sealed packets between two hubs that cannot reach each other directly. You run the server it uses; nothing is sent while the carrier is none.">
        <div className="flex flex-col gap-3">
          <CarrierForm key={`${c.kind}|${c.server ?? ""}|${c.mode}`} relay={relay} call={carrier} gated={hubGate} />
          <p className={`text-muted-foreground ${FEEDBACK}`} data-testid="carrier-consent">
            Consent: {c.consent_hash ? "accepted" : "not given yet; you are asked when the carrier is turned on."}
          </p>
          <CallOutput call={carrier} />
        </div>
      </Row>

      {c.kind === "ntfy" ? (
        <Row id="carrier-secrets" label="Access" help={SECRET_HELP}>
          <div className="flex flex-col gap-4">
            <SecretField label="Access token" name="ntfy-token" state={c.token} call={secrets} field="token" />
            <SecretField label="Topic secret" name="ntfy-topic-secret" state={c.topic_secret} call={secrets} field="topic_secret" />
            {isPlainHttp(c.server ?? "") ? <p className={`text-foreground ${FEEDBACK}`}>{TOKEN_CLEAR_NOTE}</p> : null}
            <CallOutput call={secrets} />
          </div>
        </Row>
      ) : null}
    </Panel>
  );
}
