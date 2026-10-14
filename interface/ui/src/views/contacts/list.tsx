// The contact list (os/messaging/README.md section 16): who you can write to, with pin, rename and forget. A forget is
// two steps. Names are local: the alias you set, else the operator label made from the id.
import { useState, type FormEvent } from "react";
import { redirectIfUnauthenticated } from "../../app/AuthGate";
import { announce } from "../../app/LiveRegion";
import { hrefTo } from "../../app/location";
import { Badge } from "../../components/badge";
import { Button, ButtonLink } from "../../components/button";
import { CopyButton } from "../../components/copy-button";
import { Empty } from "../../components/empty";
import { Input } from "../../components/input";
import { apiStore, getAdapter } from "../../lib/api";
import { cleanPeerText } from "../../lib/api/messages";
import type { Contact } from "../../lib/api/types";
import { Adv } from "../../lib/mode";
import { errorSentence, shortId } from "../messages/model";
import { ALIAS_MAX, aliasFromDraft, FORGET_NOTE, rowFacts, rowName } from "./model";

function RenameForm({ c, onDone }: { c: Contact; onDone: () => void }) {
  const [draft, setDraft] = useState(c.alias ?? "");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const save = async (e: FormEvent) => {
    e.preventDefault();
    if (busy) return;
    setBusy(true);
    setError(null);
    try {
      const adapter = await getAdapter();
      await adapter.contacts.patch(c.node_id, { alias: aliasFromDraft(draft) });
      await Promise.all([apiStore.refreshContacts(), apiStore.refreshThreads()]);
      announce("Renamed.");
      onDone();
    } catch (err) {
      if (redirectIfUnauthenticated(err)) return;
      setError(errorSentence(err, "Could not rename. Try again."));
    } finally {
      setBusy(false);
    }
  };

  return (
    <form onSubmit={(e) => void save(e)} className="flex flex-col gap-2">
      <label htmlFor={`alias-${c.node_id}`} className="text-[10px] uppercase tracking-widest text-muted-foreground">
        Name for {c.label}
      </label>
      <div className="flex flex-wrap items-stretch gap-2">
        <Input
          id={`alias-${c.node_id}`}
          value={draft}
          maxLength={ALIAS_MAX}
          autoComplete="off"
          spellCheck={false}
          autoFocus
          className="min-w-0 flex-1 basis-48"
          onChange={(e) => setDraft(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Escape") onDone();
          }}
        />
        <Button type="submit" size="sm" variant="primary" loading={busy}>
          save name
        </Button>
        <Button size="sm" onClick={onDone}>
          cancel
        </Button>
      </div>
      <p className="text-[11px] leading-snug text-muted-foreground">Only you see this name. Leave it empty to show {c.label}.</p>
      {error ? (
        <p role="alert" className="text-xs text-destructive">
          {error}
        </p>
      ) : null}
    </form>
  );
}

function ContactRow({ c }: { c: Contact }) {
  const [editing, setEditing] = useState(false);
  const [busy, setBusy] = useState<"pin" | "forget" | null>(null);
  const [error, setError] = useState<string | null>(null);
  const name = rowName(c);

  const run = async (kind: "pin" | "forget") => {
    setBusy(kind);
    setError(null);
    try {
      const adapter = await getAdapter();
      if (kind === "pin") await adapter.contacts.patch(c.node_id, { pinned: !c.pinned });
      else await adapter.contacts.forget(c.node_id);
      announce(kind === "forget" ? `Forgot ${name}.` : c.pinned ? `Unpinned ${name}.` : `Pinned ${name}.`);
      await Promise.all([apiStore.refreshContacts(), apiStore.refreshThreads()]);
    } catch (err) {
      if (redirectIfUnauthenticated(err)) return;
      setError(errorSentence(err, kind === "pin" ? "Could not change the pin. Try again." : "Could not forget the contact. Try again."));
    } finally {
      setBusy(null);
    }
  };

  return (
    <li className="flex flex-col gap-2 border-b border-border px-4 py-3 font-mono last:border-b-0">
      <div className="flex flex-wrap items-baseline justify-between gap-x-3 gap-y-1">
        <span className="min-w-0 break-words text-sm text-foreground">{name}</span>
        <span className="flex shrink-0 flex-wrap items-center gap-1.5">
          {c.pinned ? <Badge>pinned</Badge> : null}
          {!c.has_key ? <Badge>no card yet</Badge> : null}
        </span>
      </div>
      <p className="text-[11px] leading-snug text-muted-foreground">{rowFacts(c).join(" · ")}</p>
      <Adv>
        <div className="flex flex-col gap-1 break-all text-[11px] leading-snug text-muted-foreground">
          <span className="flex flex-wrap items-center gap-2">
            {c.bech32}
            <CopyButton text={c.bech32} ariaLabel={`Copy node id of ${name}`} />
          </span>
          <span>
            {c.label} · {shortId(c.node_id)} · trust {c.trust}
            {c.roles && c.roles.length > 0 ? ` · roles ${c.roles.map(cleanPeerText).join(", ")}` : ""}
            {c.home_hub ? ` · home hub hint ${cleanPeerText(c.home_hub)}` : ""}
          </span>
        </div>
      </Adv>
      {editing ? (
        <RenameForm c={c} onDone={() => setEditing(false)} />
      ) : (
        <div className="flex flex-wrap items-center gap-2">
          <ButtonLink size="sm" href={hrefTo(`/messages/${c.node_id}`)}>
            message<span className="sr-only"> {name}</span>
          </ButtonLink>
          <Button size="sm" onClick={() => setEditing(true)}>
            rename<span className="sr-only"> {name}</span>
          </Button>
          <Button size="sm" aria-pressed={c.pinned} loading={busy === "pin"} onClick={() => void run("pin")}>
            {c.pinned ? "unpin" : "pin"}
            <span className="sr-only"> {name}</span>
          </Button>
          <Button size="sm" variant="destructive" confirm={`forget ${name}`} loading={busy === "forget"} onClick={() => void run("forget")}>
            forget<span className="sr-only"> {name}</span>
          </Button>
        </div>
      )}
      {error ? (
        <p role="alert" className="text-xs text-destructive">
          {error}
        </p>
      ) : null}
    </li>
  );
}

export function ContactList({ contacts }: { contacts: readonly Contact[] }) {
  if (contacts.length === 0) {
    return (
      <Empty title="No contacts yet.">
        Add someone with their card or node id, on this page. Their conversations then appear under Messages. {FORGET_NOTE}
      </Empty>
    );
  }
  return (
    <>
      <ul className="border border-border">
        {contacts.map((c) => (
          <ContactRow key={c.node_id} c={c} />
        ))}
      </ul>
      <p className="max-w-prose font-mono text-[11px] leading-snug text-muted-foreground">Pinned contacts are never pruned. {FORGET_NOTE}</p>
    </>
  );
}
