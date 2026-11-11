// Settings, Relay & presence, Metered networks (os/7-relay/README.md §12): the links you marked as costing money or data, each with a
// daily cap. The list is written whole (`metered` replaces it). Turning a cap on or raising it while the carrier is on asks again: the
// node's exact words are shown first and the call repeated with their hash.
import { useId, useState, type FormEvent } from "react";
import { Button } from "../../../components/button";
import { Input } from "../../../components/input";
import { Panel } from "../../../components/panel";
import { Segmented } from "../../../components/segmented";
import type { RelayStatus } from "../../../lib/api/types";
import { PeerText } from "../../services/shared";
import { FEEDBACK, Row } from "../row";
import { useRelayCall } from "./actions";
import { CallOutput } from "./call-view";
import { METERED_HELP, meteredKey, meteredLabel, PATCH, withMetered, withoutMetered } from "./model";

const LABEL = "font-mono text-[10px] uppercase tracking-widest text-muted-foreground";

export function MeteredSection({ relay }: { relay: RelayStatus }) {
  const id = useId();
  const list = Array.isArray(relay.metered) ? relay.metered : [];
  const [by, setBy] = useState<"iface" | "ssid">("ssid");
  const [name, setName] = useState("");
  const [cap, setCap] = useState("");
  const [invalid, setInvalid] = useState<string | null>(null);
  const call = useRelayCall();
  const add = async (e: FormEvent) => {
    e.preventDefault();
    if (call.state.pending) return;
    const next = withMetered(list, by, name, cap);
    if (!next.ok) return setInvalid(next.problem);
    setInvalid(null);
    const done = await call.patch(PATCH.metered(next.list), { done: "Saved." });
    if (done.ok) {
      setName("");
      setCap("");
    }
  };
  return (
    <Panel title="metered networks" suffix="daily cap" flush>
      <Row id="metered" label="Metered networks" help={METERED_HELP}>
        <div className="flex flex-col gap-4">
          {list.length === 0 ? (
            <p className={`text-muted-foreground ${FEEDBACK}`} data-testid="metered-empty">
              None marked.
            </p>
          ) : (
            <ul className="flex flex-col gap-2" data-testid="metered-list">
              {list.map((m) => (
                <li key={meteredKey(m)} className="flex flex-wrap items-center gap-2 font-mono text-xs text-foreground" data-testid="metered-row">
                  <PeerText>{meteredLabel(m)}</PeerText>
                  <Button size="sm" confirm={`Remove ${meteredKey(m).split(":")[1] ?? "this link"}`} onClick={() => void call.patch(PATCH.metered(withoutMetered(list, meteredKey(m))), { done: "Removed." })}>
                    Remove
                  </Button>
                </li>
              ))}
            </ul>
          )}
          <form className="flex flex-col gap-3" onSubmit={(e) => void add(e)} noValidate>
            <Segmented<"iface" | "ssid"> label="Match by" value={by} onChange={setBy} options={[{ value: "ssid", label: "Wi-Fi name" }, { value: "iface", label: "network interface" }]} />
            <div className="grid gap-3 sm:grid-cols-[minmax(0,3fr)_minmax(0,2fr)]">
              <div className="flex flex-col gap-1">
                <label htmlFor={`${id}-name`} className={LABEL}>
                  {by === "ssid" ? "Wi-Fi name" : "interface"}
                </label>
                <Input id={`${id}-name`} value={name} autoComplete="off" spellCheck={false} placeholder={by === "ssid" ? "Home hotspot" : "wwan0"} onChange={(e) => setName(e.target.value)} />
              </div>
              <div className="flex flex-col gap-1">
                <label htmlFor={`${id}-cap`} className={LABEL}>
                  daily cap, MB
                </label>
                <Input id={`${id}-cap`} value={cap} inputMode="numeric" autoComplete="off" placeholder="50" onChange={(e) => setCap(e.target.value)} />
              </div>
            </div>
            <div>
              <Button type="submit" size="sm" variant="primary" arrow={false} loading={call.state.pending}>
                Add or change
              </Button>
            </div>
          </form>
          <CallOutput call={call} problem={invalid} />
        </div>
      </Row>
    </Panel>
  );
}
