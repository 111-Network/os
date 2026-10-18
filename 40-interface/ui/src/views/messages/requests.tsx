// Pending message requests (an INTRO, os/50-messaging/README.md section 7.2): the held words as text only, and what a person
// can do with them.
//
//   accept          makes the contact and delivers the held messages to the thread.
//   ignore          moves the request out of the way on THIS device. Nothing reaches the node: the request stays
//                   pending ("kept in the request list, never auto-pruned while pending, no ACK sent"), is listed under
//                   "ignored requests" and leaves the badge, and comes back when the sender writes again.
//   forget request  discards the request and its held words (DELETE /contacts/{id}) behind a two-step confirm. The
//                   sender was never ACKed and keeps its copy, so a retransmission brings the request back.
import { useState } from "react";
import { navigate } from "wouter/use-hash-location";
import { announce } from "../../app/LiveRegion";
import { redirectIfUnauthenticated } from "../../app/AuthGate";
import { hrefTo } from "../../app/location";
import { Button } from "../../components/button";
import { Disclosure } from "../../components/disclosure";
import { apiStore, getAdapter } from "../../lib/api";
import { useIgnoredRequests } from "../../lib/api/hooks";
import { isIgnored } from "../../lib/api/requests";
import type { NodeId } from "../../lib/api/types";
import { Adv } from "../../lib/mode";
import { errorSentence, FORGET_REQUEST_CONFIRM, IGNORED_NOTE, REQUEST_NOTE, requestAnnouncement, shortId, type RequestItem } from "./model";

export interface RequestRowProps {
  peer: NodeId;
  /** The operator label: a request has no alias. */
  label: string;
  /** What they wrote (untrusted, shown as text). */
  preview: string | null;
  /** The newest `received_seq` the request holds, for the ignore fold; null when it is not known (then "ignore" is not offered). */
  seq: number | null;
  /** Open the conversation after accepting (the Messages list does; the Contacts page stays put). */
  openAfterAccept?: boolean;
  /** The request is ignored on this device: "ignore" reads "show again". */
  ignored?: boolean;
}

export function RequestRow({ peer, label, preview, seq, openAfterAccept = false, ignored = false }: RequestRowProps) {
  const [busy, setBusy] = useState<"accept" | "forget" | null>(null);
  const [error, setError] = useState<string | null>(null);

  const act = async (kind: "accept" | "forget") => {
    setBusy(kind);
    setError(null);
    try {
      const adapter = await getAdapter();
      if (kind === "accept") await adapter.contacts.patch(peer, { accept: true });
      else await adapter.contacts.forget(peer);
      announce(kind === "accept" ? requestAnnouncement.accepted(label) : requestAnnouncement.forgotten(label));
      await Promise.all([apiStore.refreshContacts(), apiStore.refreshThreads()]);
      if (kind === "accept" && openAfterAccept) navigate(`/messages/${peer}`);
    } catch (err) {
      if (redirectIfUnauthenticated(err)) return;
      setError(errorSentence(err, kind === "accept" ? "Could not accept the request. Try again." : "Could not forget the request. Try again."));
    } finally {
      setBusy(null);
    }
  };

  // Ignoring is a fold this page keeps for itself: no daemon call (§7.2).
  const toggleIgnored = () => {
    if (ignored) {
      apiStore.restoreRequest(peer);
      announce(requestAnnouncement.restored(label));
    } else if (seq !== null) {
      apiStore.ignoreRequest(peer, seq);
      announce(requestAnnouncement.ignored(label));
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
      {preview !== null ? (
        <p dir="auto" className="max-w-prose break-words text-xs leading-relaxed text-muted-foreground">
          {preview}
        </p>
      ) : null}
      <div className="flex flex-wrap items-center gap-2">
        <Button size="sm" variant="primary" loading={busy === "accept"} aria-disabled={busy === "forget" || undefined} onClick={() => void act("accept")}>
          accept
          <span className="sr-only"> the request from {label}</span>
        </Button>
        {ignored || seq !== null ? (
          <Button size="sm" onClick={toggleIgnored}>
            {ignored ? "show again" : "ignore"}
            <span className="sr-only"> the request from {label}</span>
          </Button>
        ) : null}
        <Button size="sm" variant="destructive" confirm={FORGET_REQUEST_CONFIRM} loading={busy === "forget"} aria-disabled={busy === "accept" || undefined} onClick={() => void act("forget")}>
          forget request
          <span className="sr-only"> from {label}</span>
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

export interface RequestsSectionProps {
  items: readonly RequestItem[];
  headingId: string;
  openAfterAccept?: boolean;
}

/**
 * The "message requests" block shared by Messages and Contacts: the requests that need an answer, then the ones ignored on
 * this device folded under a disclosure (still pending on the node, still acceptable, one click to bring back).
 */
export function RequestsSection({ items, headingId, openAfterAccept = false }: RequestsSectionProps) {
  const ignored = useIgnoredRequests();
  if (items.length === 0) return null;
  const folded = items.filter((i) => isIgnored(ignored, i.peer, i.seq));
  const shown = items.filter((i) => !isIgnored(ignored, i.peer, i.seq));
  const row = (i: RequestItem, isFolded: boolean) => <RequestRow key={i.peer} peer={i.peer} label={i.label} preview={i.preview} seq={i.seq} openAfterAccept={openAfterAccept} ignored={isFolded} />;
  return (
    <section aria-labelledby={headingId} className="flex flex-col gap-2">
      <h2 id={headingId} className="font-mono text-[10px] uppercase tracking-widest text-muted-foreground">
        message requests ({shown.length})
      </h2>
      <RequestNote />
      {shown.length > 0 ? (
        <ul className="border border-border">{shown.map((i) => row(i, false))}</ul>
      ) : (
        <p className="font-mono text-xs text-muted-foreground">No new requests.</p>
      )}
      {folded.length > 0 ? (
        <Disclosure summary={`ignored requests (${folded.length})`}>
          <div className="flex flex-col gap-2">
            <p className="max-w-prose font-mono text-xs leading-relaxed text-muted-foreground">{IGNORED_NOTE}</p>
            <ul className="border border-border">{folded.map((i) => row(i, true))}</ul>
          </div>
        </Disclosure>
      ) : null}
    </section>
  );
}
