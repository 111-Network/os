// Settings, Relay & presence, Updates (os/7-relay/README.md §9, §9.3, §9.5 T4, §12): the order the origins are tried in and what each last
// answered, the internet check switch, "serve updates for" (this hub as an origin, and the targets it holds), the `111 update --from`
// instruction for a node with no network, and "newest release seen: <calver> from <origin>". Every url and detail is a node's or a
// server's string: cleaned and drawn as text.
import { useEffect, useId, useState, type FormEvent } from "react";
import { Button } from "../../../components/button";
import { CopyButton } from "../../../components/copy-button";
import { Input } from "../../../components/input";
import { Panel } from "../../../components/panel";
import { formatBytes } from "../../../lib/api/fabric";
import { getRelay } from "../../../lib/api";
import type { RelayStatus, UpdateOrigin } from "../../../lib/api/types";
import { plain } from "../../nearby/model";
import { ageAgoText } from "../../services/model";
import { PeerText } from "../../services/shared";
import { FEEDBACK, Row } from "../row";
import { useRelayCall } from "./actions";
import { CallOutput } from "./call-view";
import {
  answerText,
  HUB_ONLY_TEXT,
  INTERNET_CHECK_HELP,
  newestSeen,
  newestSeenText,
  originKindWord,
  originName,
  PATCH,
  problemDetail,
  SERVE_HELP,
  TARGET_TRIPLE,
  UPDATE_FROM_COMMAND,
  UPDATE_FROM_HELP,
} from "./model";
import { SwitchRow } from "./switch";

/** `GET /update/origins`, read when the page opens and again when a setting that changes it moves. No poll. */
function useOrigins(key: string): UpdateOrigin[] | null {
  const [origins, setOrigins] = useState<UpdateOrigin[] | null>(null);
  useEffect(() => {
    let alive = true;
    getRelay()
      .then((r) => r.updateOrigins())
      .then((o) => alive && setOrigins(o))
      .catch(() => alive && setOrigins((prev) => prev)); // an older node has none: the list stays as it was
    return () => {
      alive = false;
    };
  }, [key]);
  return origins;
}

function TargetsRow({ targets, gated }: { targets: readonly string[]; gated: boolean }) {
  const id = useId();
  const [value, setValue] = useState("");
  const [invalid, setInvalid] = useState<string | null>(null);
  const call = useRelayCall();
  // The node refuses to drop its own target; whatever it says about a target is shown as it wrote it.
  const write = (next: string[], done: string) => call.patch(PATCH.targets(next), { done, errorText: problemDetail });
  const add = async (e: FormEvent) => {
    e.preventDefault();
    if (gated || call.state.pending) return;
    const t = value.trim().toLowerCase();
    if (!TARGET_TRIPLE.test(t)) return setInvalid("Type a target such as aarch64-unknown-linux-musl.");
    if (targets.includes(t)) return setInvalid("Already on the list.");
    setInvalid(null);
    if ((await write([...targets, t], `${t} added.`)).ok) setValue("");
  };
  return (
    <div className="flex flex-col gap-3" data-testid="origin-targets">
      <ul className="flex flex-col gap-1">
        {targets.map((t) => (
          <li key={t} className="flex flex-wrap items-center gap-2 font-mono text-xs text-foreground">
            <PeerText>{plain(t, 60)}</PeerText>
            <Button size="sm" confirm={`Stop holding ${plain(t, 40)}`} aria-disabled={gated || call.state.pending || undefined} onClick={() => !gated && void write(targets.filter((x) => x !== t), `${t} removed.`)}>
              Remove
            </Button>
          </li>
        ))}
      </ul>
      <form className="flex flex-wrap items-end gap-2" onSubmit={(e) => void add(e)} noValidate>
        <div className="flex min-w-0 flex-1 basis-56 flex-col gap-1">
          <label htmlFor={`${id}-t`} className="font-mono text-[10px] uppercase tracking-widest text-muted-foreground">
            hold files for another target
          </label>
          <Input id={`${id}-t`} value={value} autoComplete="off" spellCheck={false} placeholder="aarch64-unknown-linux-musl" onChange={(e) => setValue(e.target.value)} />
        </div>
        <Button size="sm" type="submit" loading={call.state.pending} aria-disabled={gated || undefined}>
          Add
        </Button>
      </form>
      <CallOutput call={call} problem={invalid} />
    </div>
  );
}

export function UpdatesSection({ relay, hubGate }: { relay: RelayStatus; hubGate: boolean }) {
  const origins = useOrigins(`${relay.updates.check_internet}|${relay.origin.serve}`);
  const seen = newestSeen(origins ?? []);
  const targets = Array.isArray(relay.origin.targets) ? relay.origin.targets : [];
  return (
    <Panel title="updates" suffix="origins" flush>
      <Row id="origins" label="Where updates come from" help="Tried in this order. Your hub counts only while it serves update files. Every file is checked against the 111 release key whichever origin it came from.">
        {origins === null ? (
          <p className={`text-muted-foreground ${FEEDBACK}`}>Not reported yet.</p>
        ) : origins.length === 0 ? (
          <p className={`text-muted-foreground ${FEEDBACK}`}>No origins.</p>
        ) : (
          <ol className="flex flex-col gap-2" data-testid="origin-list">
            {origins.map((o, i) => (
              <li key={`${o.kind}|${o.url}`} className="flex flex-col font-mono text-xs leading-relaxed" data-testid="origin-row">
                <span className="text-foreground">
                  {i + 1}. {originKindWord(o.kind)}: <PeerText>{plain(o.url, 120)}</PeerText>
                </span>
                <span className="text-muted-foreground">
                  {answerText(o.last_answer, (s) => ageAgoText(s) ?? "")}
                  {o.newest_seen ? `; newest release ${plain(o.newest_seen, 24)}` : ""} (<PeerText>{originName(o)}</PeerText>)
                </span>
              </li>
            ))}
          </ol>
        )}
        <p className={`mt-3 text-foreground ${FEEDBACK}`} data-testid="newest-seen">
          {newestSeenText(seen)}
        </p>
      </Row>
      <SwitchRow id="internet-check" label="Check the internet" help={INTERNET_CHECK_HELP} on={relay.updates.check_internet} patch={PATCH.checkInternet} />
      <SwitchRow
        id="origin-serve"
        label="Serve update files"
        help={hubGate ? `${SERVE_HELP} ${HUB_ONLY_TEXT}` : `${SERVE_HELP} Holds ${formatBytes(relay.origin.bytes)} now.`}
        on={relay.origin.serve}
        disabled={hubGate}
        patch={PATCH.originServe}
      />
      <Row id="serve-targets" label="Serve updates for" help="The kinds of device this hub keeps update files for. The hub keeps its own, and says so if you try to remove it.">
        <TargetsRow targets={targets} gated={hubGate} />
      </Row>
      <Row id="update-from" label="No network at all" help={UPDATE_FROM_HELP}>
        <div className="flex flex-wrap items-center gap-2">
          <code className="whitespace-pre-wrap break-words border border-border bg-secondary/20 px-2 py-1 font-mono text-xs text-foreground" data-testid="update-from-command">
            {UPDATE_FROM_COMMAND}
          </code>
          <CopyButton text={UPDATE_FROM_COMMAND} label="copy command" ariaLabel="Copy the update command" />
        </div>
      </Row>
    </Panel>
  );
}
