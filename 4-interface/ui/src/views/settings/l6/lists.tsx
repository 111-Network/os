// Recipient lists per network (os/60-transport/README.md §6): local, destination-only groups of native addresses, entered, pasted or
// picked from the announces Nearby heard. Saving replaces the whole set (`PATCH /bridge/settings recipient_lists`). Names are the
// person's own; the addresses are checked against the network's `recipient_syntax` before they are added.
import { useEffect, useState } from "react";
import { announce } from "../../../app/LiveRegion";
import { Button } from "../../../components/button";
import { Input } from "../../../components/input";
import { Textarea } from "../../../components/textarea";
import { apiStore } from "../../../lib/api";
import { addressProblem, membersText, parseAddresses } from "../../../lib/api/bridge";
import { useNearby } from "../../../lib/api/hooks";
import type { BridgeSettings, Network, RecipientList } from "../../../lib/api/types";
import { plain } from "../../messages/net/model";
import { useAction } from "../essentials";
import { Feedback, FEEDBACK, Row } from "../row";
import { LISTS_HELP, MAX_LIST_MEMBERS, MAX_LIST_NAME, listId, sameLists } from "./model";

function ListEditor({ list, network, onChange, onRemove }: { list: RecipientList; network: Network; onChange: (l: RecipientList) => void; onRemove: () => void }) {
  const [paste, setPaste] = useState("");
  const [problem, setProblem] = useState<string | null>(null);
  const nearby = useNearby();
  const syntax = network.caps.recipient_syntax;
  const heard = (nearby ?? []).filter((s) => s.kind === "reticulum").flatMap((s) => s.nodes);

  const add = (addresses: string[]) => {
    const fresh = addresses.filter((a) => !list.members.includes(a));
    const bad = fresh.map((a) => addressProblem(syntax, a)).find((p) => p !== null);
    if (bad) return setProblem(bad);
    if (list.members.length + fresh.length > MAX_LIST_MEMBERS) return setProblem(`A list holds at most ${MAX_LIST_MEMBERS} addresses.`);
    setProblem(null);
    onChange({ ...list, members: [...list.members, ...fresh] });
  };
  return (
    <li className="flex flex-col gap-3 border border-border p-3" data-testid={`list-${list.id}`}>
      <div className="flex flex-wrap items-end gap-2">
        <label className="flex min-w-0 flex-1 flex-col gap-1">
          <span className="font-mono text-[10px] uppercase tracking-widest text-foreground">name</span>
          <Input value={list.name} maxLength={MAX_LIST_NAME} autoComplete="off" onChange={(e) => onChange({ ...list, name: e.target.value })} />
        </label>
        <Button size="sm" variant="destructive" confirm={`Remove the list ${plain(list.name, 30)}`} onClick={onRemove}>
          remove list
        </Button>
      </div>
      <p className={`text-muted-foreground ${FEEDBACK}`}>{membersText(list.members.length)}</p>
      {list.members.length > 0 ? (
        <ul className="flex flex-col gap-1">
          {list.members.map((m) => (
            <li key={m} className="flex flex-wrap items-center justify-between gap-2 font-mono text-xs">
              <span className="min-w-0 break-all text-foreground">{m}</span>
              <Button size="sm" aria-label={`Remove ${m}`} onClick={() => onChange({ ...list, members: list.members.filter((x) => x !== m) })}>
                remove
              </Button>
            </li>
          ))}
        </ul>
      ) : null}
      <label className="flex flex-col gap-1">
        <span className="font-mono text-[10px] uppercase tracking-widest text-foreground">add addresses ({syntax?.label ?? "address"})</span>
        <Textarea rows={2} value={paste} autoComplete="off" spellCheck={false} placeholder={syntax?.example ?? ""} onChange={(e) => {
            setPaste(e.target.value);
            setProblem(null);
          }} />
      </label>
      <div className="flex flex-wrap items-center gap-2">
        <Button
          size="sm"
          onClick={() => {
            const a = parseAddresses(paste);
            if (a.length === 0) return setProblem("Type or paste an address first.");
            add(a);
            if (a.every((x) => addressProblem(syntax, x) === null)) setPaste("");
          }}
        >
          add
        </Button>
      </div>
      {heard.length > 0 ? (
        <div className="flex flex-col gap-1" data-testid="heard">
          <p className="font-mono text-[10px] uppercase tracking-widest text-muted-foreground">heard by this node</p>
          <ul className="flex flex-col gap-1">
            {heard.map((n) => (
              <li key={n.id} className="flex flex-wrap items-center justify-between gap-2 font-mono text-xs">
                <span dir="auto" className="min-w-0 break-words text-foreground [unicode-bidi:isolate]">
                  {plain(n.long_name || n.short_name || n.id, 40)} <span className="text-muted-foreground">{n.id.slice(0, 8)}…</span>
                </span>
                <Button size="sm" aria-disabled={list.members.includes(n.id) || undefined} onClick={() => add([n.id])} aria-label={`Add ${plain(n.long_name || n.id, 40)} to ${plain(list.name, 30)}`}>
                  add
                </Button>
              </li>
            ))}
          </ul>
        </div>
      ) : null}
      <Feedback text={problem} problem />
    </li>
  );
}

/** Lists for the networks that take typed addresses and can hold a group. */
export function ListsRow({ settings, networks }: { settings: BridgeSettings | null; networks: readonly Network[] }) {
  const mine = networks.filter((n) => n.caps.recipient_syntax && n.caps.kinds.includes("group") && n.state !== "disabled");
  const saved = settings?.recipient_lists ?? [];
  const [draft, setDraft] = useState<RecipientList[] | null>(null);
  const [action, run] = useAction();
  const [saved_, setSaved] = useState(false);
  const lists = draft ?? saved;
  // A snapshot that arrives while there is nothing unsaved replaces the draft.
  useEffect(() => {
    if (draft && sameLists(draft, saved)) setDraft(null);
  }, [saved, draft]);
  if (mine.length === 0) {
    return (
      <Row id="lists" label="Recipient lists" help={LISTS_HELP}>
        <p className={`text-muted-foreground ${FEEDBACK}`}>No network that takes typed addresses is on. Turn on Reticulum above to make lists.</p>
      </Row>
    );
  }
  const set = (next: RecipientList[]) => {
    setSaved(false);
    setDraft(next);
  };
  const save = async () => {
    const out = await run((a) => a.bridge.patchSettings({ recipient_lists: lists }));
    if (!out) return;
    apiStore.adoptBridgeSettings(out);
    void apiStore.refreshConversations("lxmf");
    setDraft(null);
    setSaved(true);
    announce("Recipient lists saved.");
  };
  const dirty = draft !== null && !sameLists(draft, saved);
  return (
    <Row id="lists" label="Recipient lists" help={LISTS_HELP}>
      <div className="flex flex-col gap-4">
        {mine.map((n) => (
          <div key={n.id} className="flex flex-col gap-2" data-testid={`lists-${n.id}`}>
            <p className="font-mono text-[10px] uppercase tracking-widest text-muted-foreground">{n.label}</p>
            <ul className="flex flex-col gap-3">
              {lists
                .filter((l) => l.network === n.id)
                .map((l) => (
                  <ListEditor key={l.id} list={l} network={n} onChange={(next) => set(lists.map((x) => (x.id === l.id ? next : x)))} onRemove={() => set(lists.filter((x) => x.id !== l.id))} />
                ))}
            </ul>
            <div>
              <Button size="sm" onClick={() => set([...lists, { id: listId("list", lists.map((l) => l.id)), network: n.id, name: "New list", members: [] }])}>
                add a list
              </Button>
            </div>
          </div>
        ))}
        <div className="flex flex-wrap items-center gap-2">
          <Button size="sm" variant="primary" arrow={false} loading={action.pending} aria-disabled={!dirty || undefined} onClick={() => void save()} data-testid="lists-save">
            save lists
          </Button>
          {dirty ? <span className={`text-muted-foreground ${FEEDBACK}`}>Unsaved changes.</span> : null}
        </div>
        <Feedback text={action.error ?? (saved_ ? "Saved." : null)} problem={action.error !== null} />
      </div>
    </Row>
  );
}
