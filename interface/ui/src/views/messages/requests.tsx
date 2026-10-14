// A pending message request (an INTRO, os/messaging/README.md section 7.2): the held words as text only, and the
// two things a person can do. Accept makes the contact and delivers the held messages to the thread. Ignore discards
// the request without sending anything back (no ACK, the sender is not told); it is a two-step action because the
// node forgets the pending contact.
import { useState } from "react";
import { navigate } from "wouter/use-hash-location";
import { announce } from "../../app/LiveRegion";
import { redirectIfUnauthenticated } from "../../app/AuthGate";
import { hrefTo } from "../../app/location";
import { Button } from "../../components/button";
import { apiStore, getAdapter } from "../../lib/api";
import type { NodeId } from "../../lib/api/types";
import { errorSentence, REQUEST_NOTE, shortId } from "./model";
import { Adv } from "../../lib/mode";

export interface RequestRowProps {
  peer: NodeId;
  /** The operator label: a request has no alias. */
  label: string;
  /** What they wrote (untrusted, shown as text). */
  preview: string | null;
  /** Open the conversation after accepting (the Messages list does; the Contacts page stays put). */
  openAfterAccept?: boolean;
}

export function RequestRow({ peer, label, preview, openAfterAccept = false }: RequestRowProps) {
  const [busy, setBusy] = useState<"accept" | "ignore" | null>(null);
  const [error, setError] = useState<string | null>(null);

  const act = async (kind: "accept" | "ignore") => {
    setBusy(kind);
    setError(null);
    try {
      const adapter = await getAdapter();
      if (kind === "accept") await adapter.contacts.patch(peer, { accept: true });
      else await adapter.contacts.forget(peer);
      announce(kind === "accept" ? `Accepted the request from ${label}.` : `Ignored the request from ${label}.`);
      await Promise.all([apiStore.refreshContacts(), apiStore.refreshThreads()]);
      if (kind === "accept" && openAfterAccept) navigate(`/messages/${peer}`);
    } catch (err) {
      if (redirectIfUnauthenticated(err)) return;
      setError(errorSentence(err, kind === "accept" ? "Could not accept the request. Try again." : "Could not ignore the request. Try again."));
    } finally {
      setBusy(null);
    }
  };

  return (
    <li className="flex flex-col gap-2 border-b border-border px-4 py-3 font-mono last:border-b-0">
      <div className="flex flex-wrap items-baseline justify-between gap-x-3 gap-y-1">
        <a href={hrefTo(`/messages/${peer}`)} className="min-w-0 break-words text-sm text-foreground underline-offset-4 hover:underline focus-visible:ring-2 focus-visible:ring-foreground focus-visible:outline-none">
          {label}
          <span className="sr-only"> (a message request, open it to read)</span>
        </a>
        <Adv>
          <span className="text-[11px] text-muted-foreground" title={peer}>
            {shortId(peer)}
          </span>
        </Adv>
      </div>
      {preview !== null ? <p className="max-w-prose break-words text-xs leading-relaxed text-muted-foreground">{preview}</p> : null}
      <div className="flex flex-wrap items-center gap-2">
        <Button size="sm" variant="primary" loading={busy === "accept"} aria-disabled={busy === "ignore" || undefined} onClick={() => void act("accept")}>
          accept
          <span className="sr-only"> the request from {label}</span>
        </Button>
        <Button size="sm" variant="destructive" confirm="ignore this request" loading={busy === "ignore"} aria-disabled={busy === "accept" || undefined} onClick={() => void act("ignore")}>
          ignore
          <span className="sr-only"> the request from {label}</span>
        </Button>
      </div>
      {error ? (
        <p role="alert" className="text-xs text-destructive">
          {error}
        </p>
      ) : null}
    </li>
  );
}

/** The framing sentence above a list of requests. */
export function RequestNote() {
  return <p className="max-w-prose font-mono text-xs leading-relaxed text-muted-foreground">{REQUEST_NOTE}</p>;
}
