// One conversation (os/messaging/README.md section 16): messages in the order they arrived here, each with its own
// delivery evidence in the exact words of section 9, the reason when it waits, the time it was received here, and the
// sender's own claim behind a disclosure, marked unverified. Peer text is rendered as text only.
import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState, type FormEvent } from "react";
import { announce } from "../../app/LiveRegion";
import { redirectIfUnauthenticated } from "../../app/AuthGate";
import { useNow } from "../../app/hooks";
import { hrefTo } from "../../app/location";
import { Button, ButtonLink } from "../../components/button";
import { CopyButton } from "../../components/copy-button";
import { Disclosure } from "../../components/disclosure";
import { Empty } from "../../components/empty";
import { JsonBlock } from "../../components/json-block";
import { Panel } from "../../components/panel";
import { SectionHeader } from "../../components/section-header";
import { Textarea } from "../../components/textarea";
import { apiStore, getAdapter } from "../../lib/api";
import { contactName } from "../../lib/api/contacts";
import { useApiMeta, useContacts, useIgnoredRequests, useThread, useThreads } from "../../lib/api/hooks";
import { cleanDeep, cleanPeerText, isCancellable, isCancelled, isRetryable, newestIncomingSeq, newestReceivedSeq, THREAD_MAX_LIMIT, THREAD_PAGE_LIMIT } from "../../lib/api/messages";
import { isIgnored, requestSeq } from "../../lib/api/requests";
import type { Message, NodeId } from "../../lib/api/types";
import { cx } from "../../lib/cx";
import { Adv } from "../../lib/mode";
import { MessagingBanner } from "./banner";
import { StateGlyph } from "./glyph";
import {
  CANCEL_CONFIRM,
  CANCEL_UNSUPPORTED_TEXT,
  CANCELLED_ANNOUNCEMENT,
  claimText,
  composeGate,
  type ComposeGate,
  draftProblem,
  errorSentence,
  evidenceName,
  IGNORED_HERE_NOTE,
  MAX_COMPOSE_CHARS,
  messageStateCopy,
  reasonText,
  timeLabel,
  whenText,
} from "./model";
import { RequestRow } from "./requests";

const FOLLOW_SLACK_PX = 24;

/**
 * Does the adapter in use carry `messages.cancel` (`POST /messages/{id}/cancel`)? The control is offered only where it does:
 * a node or mock built before the route existed has none, and a button that cannot work is worse than no button.
 */
function useCanCancel(): boolean {
  const [can, setCan] = useState(false);
  useEffect(() => {
    let live = true;
    getAdapter().then(
      (a) => {
        if (live) setCan(typeof a.messages.cancel === "function");
      },
      () => undefined,
    );
    return () => {
      live = false;
    };
  }, []);
  return can;
}

export function MessageRow({ m, now, onRetry, onCancel }: { m: Message; now: number; onRetry: (id: string) => void; onCancel?: ((id: string) => void) | undefined }) {
  const own = m.dir === "own";
  const reason = own ? reasonText(m) : null;
  const claim = claimText(m, now);
  const wire = m.msg_id;
  return (
    <li className={cx("flex flex-col gap-1", own ? "items-end" : "items-start")} data-testid={`msg-${m.id}`}>
      <div dir="auto" className={cx("max-w-[85%] whitespace-pre-wrap break-words border px-3 py-2 text-sm leading-relaxed", own ? "border-border bg-secondary text-foreground" : "border-border text-foreground")}>
        {cleanPeerText(m.text)}
      </div>
      {reason ? <p className="max-w-[85%] text-end text-xs leading-snug text-muted-foreground">{reason}</p> : null}
      <Disclosure
        className="max-w-[85%] text-[11px] text-muted-foreground"
        summary={
          <span className="flex flex-wrap items-center gap-x-3 gap-y-1">
            {own ? <StateGlyph state={m.state} copy={messageStateCopy(m)} label={isCancelled(m) ? "cancelled" : undefined} /> : null}
            <span>
              {timeLabel(m)} {whenText(m.at, now)}
            </span>
            <span className="sr-only">(show details)</span>
          </span>
        }
      >
        <div className="flex flex-col gap-1 border border-border p-3 leading-relaxed">
          {own ? <p>{messageStateCopy(m)}</p> : <p>Arrived on this node. Messages are ordered by when they arrived here.</p>}
          {claim ? (
            <p>
              <span className="uppercase tracking-widest">unverified sender time</span> {claim}
            </p>
          ) : null}
        </div>
      </Disclosure>
      <Adv>
        <p className="max-w-[85%] break-all text-[11px] leading-snug text-muted-foreground" data-testid={`raw-${m.id}`}>
          {m.id}
          {wire ? ` · msg_id stream ${wire.stream} epoch ${wire.epoch} n ${wire.n}` : ""} · seq {m.local_seq}
          {m.received_seq !== null ? ` · received seq ${m.received_seq}` : ""}
          {" · what this state means: "}
          {evidenceName(m)}
        </p>
      </Adv>
      {isRetryable(m) || (onCancel && isCancellable(m)) ? (
        <div className="flex flex-wrap items-center justify-end gap-2">
          {isRetryable(m) ? (
            <Button size="sm" onClick={() => onRetry(m.id)} aria-label={`Retry sending: ${cleanPeerText(m.text).slice(0, 40)}`}>
              retry now
            </Button>
          ) : null}
          {onCancel && isCancellable(m) ? (
            <Button size="sm" confirm={CANCEL_CONFIRM} onClick={() => onCancel(m.id)} aria-label={`Cancel sending: ${cleanPeerText(m.text).slice(0, 40)}`} data-testid={`cancel-${m.id}`}>
              cancel sending
            </Button>
          ) : null}
        </div>
      ) : null}
    </li>
  );
}

function Composer({ peer, onSent, hint }: { peer: NodeId; onSent: () => void; hint: string | null }) {
  const [value, setValue] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const ref = useRef<HTMLTextAreaElement>(null);

  const submit = async (e?: FormEvent) => {
    e?.preventDefault();
    if (busy) return;
    const problem = draftProblem(value);
    if (problem) {
      setError(problem);
      return;
    }
    setBusy(true);
    setError(null);
    try {
      const adapter = await getAdapter();
      await adapter.messages.send({ to: peer, text: value });
      setValue("");
      onSent();
      announce("Message queued on this node.");
    } catch (err) {
      if (redirectIfUnauthenticated(err)) return;
      setError(errorSentence(err, "The message could not be queued. Try again."));
    } finally {
      setBusy(false);
      ref.current?.focus();
    }
  };

  return (
    <form className="flex flex-col gap-2 border-t border-border p-3" onSubmit={(e) => void submit(e)}>
      <div className="flex items-end gap-2">
        <Textarea
          ref={ref}
          rows={2}
          value={value}
          maxLength={MAX_COMPOSE_CHARS}
          aria-label="Message"
          aria-describedby="composer-note"
          placeholder="Write a message"
          autoComplete="off"
          className="min-h-11 min-w-0 flex-1 resize-none! overflow-y-auto"
          onChange={(e) => {
            setValue(e.target.value);
            setError(null);
          }}
          onKeyDown={(e) => {
            if (e.key !== "Enter" || e.shiftKey || e.nativeEvent.isComposing) return;
            e.preventDefault();
            void submit();
          }}
        />
        <Button type="submit" size="sm" variant="primary" loading={busy} aria-disabled={value.trim() === "" || undefined}>
          send
        </Button>
      </div>
      <p id="composer-note" className="text-[11px] leading-snug text-muted-foreground">
        Enter sends, Shift and Enter adds a line. Text only for now: attachments arrive next. {hint ?? ""}
      </p>
      {error ? (
        <p role="alert" className="text-xs text-destructive">
          {error}
        </p>
      ) : null}
    </form>
  );
}

export default function ThreadView({ peer }: { peer: NodeId }) {
  const [limit, setLimit] = useState(THREAD_PAGE_LIMIT);
  const slot = useThread(peer, limit);
  const contacts = useContacts();
  const contact = contacts?.find((c) => c.node_id === peer) ?? null;
  const threads = useThreads();
  const ignoredRequests = useIgnoredRequests();
  const { ready } = useApiMeta();
  const now = useNow(true, 30_000);
  const canCancel = useCanCancel();
  const [cancelError, setCancelError] = useState<string | null>(null);
  const page = slot.page;

  const thread = threads?.find((t) => t.peer === peer) ?? null;
  const name = contact ? contactName(contact) : (thread?.alias ?? thread?.label ?? `${peer.slice(0, 8)}…`);
  const gate: ComposeGate = contacts === null ? { ok: false, why: "Loading your contacts." } : composeGate(contact, page?.request === true);

  // --- follow the newest message unless the reader scrolled up ---------------------------------
  const listRef = useRef<HTMLDivElement>(null);
  const pinned = useRef(true);
  const onScroll = () => {
    const el = listRef.current;
    if (el) pinned.current = el.scrollHeight - el.scrollTop - el.clientHeight < FOLLOW_SLACK_PX;
  };
  useLayoutEffect(() => {
    const el = listRef.current;
    if (el && pinned.current) el.scrollTop = el.scrollHeight;
  }, [page?.messages.length]);

  // --- mark read: the read mark only moves forward, and a pending request is never marked ---------
  const newest = page ? newestIncomingSeq(page.messages) : null;
  const unread = thread?.unread ?? null;
  const marked = useRef(0);
  const isRequest = page?.request === true || contact?.request === true;
  useEffect(() => {
    if (newest === null || isRequest || !contact || unread === 0 || newest <= marked.current) return;
    marked.current = newest;
    void (async () => {
      try {
        const adapter = await getAdapter();
        await adapter.contacts.patch(peer, { read_seq: newest });
        void apiStore.refreshThreads();
      } catch {
        marked.current = 0; // try again on the next change
      }
    })();
  }, [newest, isRequest, contact, unread, peer]);

  // --- tell a screen reader about a new incoming message (not the ones already there) -------------
  const seenIn = useRef<number | null>(null);
  useEffect(() => {
    if (newest === null) return;
    if (seenIn.current !== null && newest > seenIn.current) announce(`New message from ${name}.`);
    seenIn.current = newest;
  }, [newest, name]);

  const retry = useCallback(
    async (id: string) => {
      try {
        const adapter = await getAdapter();
        await adapter.messages.retry(id);
        void apiStore.loadThread(peer, limit);
      } catch (err) {
        if (redirectIfUnauthenticated(err)) return;
        announce(errorSentence(err, "Could not retry that message."));
      }
    },
    [peer, limit],
  );

  const cancel = useCallback(
    async (id: string) => {
      setCancelError(null);
      try {
        const adapter = await getAdapter();
        if (!adapter.messages.cancel) {
          setCancelError(CANCEL_UNSUPPORTED_TEXT);
          return;
        }
        await adapter.messages.cancel(id);
        announce(CANCELLED_ANNOUNCEMENT);
        void apiStore.loadThread(peer, limit);
        void apiStore.refreshThreads();
      } catch (err) {
        if (redirectIfUnauthenticated(err)) return;
        // A 409 means it left (or was never ours to cancel) while the button was on screen: the node's own words say which.
        setCancelError(errorSentence(err, "Could not cancel that message."));
        void apiStore.loadThread(peer, limit);
      }
    },
    [peer, limit],
  );

  // Peer text goes into the Advanced JSON block cleaned: JSON.stringify leaves U+202E and the isolates unescaped.
  const raw = useMemo(() => (page ? cleanDeep({ peer: page.peer, request: page.request, more: page.more, count: page.messages.length, messages: page.messages.slice(-20) }) : null), [page]);

  // The request's newest receipt (what "ignore" is keyed on): the open page is the freshest source, then the list.
  const requestNewest = page ? newestReceivedSeq(page.messages) ?? (thread ? requestSeq(thread) : 0) : thread ? requestSeq(thread) : null;
  const requestIgnored = isIgnored(ignoredRequests, peer, requestNewest);

  const back = (
    <ButtonLink href={hrefTo("/messages")} size="sm">
      All messages
    </ButtonLink>
  );

  if (!page && slot.state === "loading") {
    return (
      <div className="flex flex-col gap-6">
        <SectionHeader kicker="conversation" title={name} level={1} end={back} />
        <p role="status" className="font-mono text-xs text-muted-foreground">
          Loading conversation
        </p>
      </div>
    );
  }

  if (!page && !contact) {
    return (
      <div className="flex flex-col gap-6">
        <SectionHeader kicker="conversation" title="Conversation" level={1} end={back} />
        <Empty
          kicker={slot.state === "missing" ? "not found" : "unavailable"}
          title={slot.state === "missing" ? "No conversation with this id" : "The conversation could not be loaded"}
          action={
            <ButtonLink href={hrefTo("/messages/contacts")} size="sm">
              Contacts
            </ButtonLink>
          }
        >
          {slot.state === "missing" && ready ? "Nothing was sent to or received from this node. Add them from Contacts to write to them." : "The node did not answer. The conversation appears here as soon as it does."}
        </Empty>
      </div>
    );
  }

  const messages = page?.messages ?? [];
  return (
    <div className="flex flex-col gap-6">
      <SectionHeader kicker="conversation" title={name} level={1} end={back} />
      <MessagingBanner />
      <Adv>
        <p className="flex flex-wrap items-center gap-2 break-all font-mono text-[11px] text-muted-foreground">
          <span>{contact?.bech32 ?? peer}</span>
          <CopyButton text={contact?.bech32 ?? peer} ariaLabel="Copy node id" />
        </p>
      </Adv>

      {page?.request || contact?.request ? (
        <div className="border border-border">
          <ul>
            <RequestRow peer={peer} label={thread?.label ?? contact?.label ?? peer.slice(0, 8)} preview={null} seq={requestNewest} ignored={requestIgnored} />
          </ul>
          {requestIgnored ? <p className="border-t border-border px-4 py-2 font-mono text-xs leading-relaxed text-muted-foreground">{IGNORED_HERE_NOTE}</p> : null}
        </div>
      ) : null}

      <Panel title="conversation" suffix={name} flush className="flex flex-col" bodyClassName="flex min-h-0 flex-1 flex-col font-mono">
        <div
          ref={listRef}
          role="log"
          aria-live="off"
          aria-label={`Conversation with ${name}`}
          tabIndex={0}
          onScroll={onScroll}
          className="flex max-h-[60dvh] min-h-48 flex-col gap-4 overflow-y-auto px-4 py-4 focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-foreground focus-visible:outline-none"
        >
          {page?.more && limit < THREAD_MAX_LIMIT ? (
            <div className="flex justify-center">
              <Button size="sm" onClick={() => setLimit((l) => Math.min(l + THREAD_PAGE_LIMIT, THREAD_MAX_LIMIT))}>
                show earlier messages
              </Button>
            </div>
          ) : null}
          {messages.length === 0 ? (
            <p className="m-auto max-w-prose text-center text-xs leading-relaxed text-muted-foreground">
              Nothing here yet. {gate.ok ? "What you write is kept on this node until a path to them opens; the state under each message says how far it got." : ""}
            </p>
          ) : (
            <ul className="flex flex-col gap-4">
              {messages.map((m) => (
                <MessageRow key={m.id} m={m} now={now} onRetry={(id) => void retry(id)} onCancel={canCancel ? (id) => void cancel(id) : undefined} />
              ))}
            </ul>
          )}
        </div>
        {cancelError ? (
          <p role="alert" className="border-t border-border px-4 py-2 text-xs text-destructive" data-testid="cancel-error">
            {cancelError}
          </p>
        ) : null}
        {slot.state === "error" ? (
          <p role="status" className="border-t border-border px-4 py-2 text-xs text-muted-foreground">
            The latest messages could not be loaded.{" "}
            <button type="button" className="underline underline-offset-4" onClick={() => void apiStore.loadThread(peer, limit)}>
              try again
            </button>
          </p>
        ) : null}
        {gate.ok ? (
          <Composer
            peer={peer}
            hint={gate.hint}
            onSent={() => {
              pinned.current = true;
              void apiStore.loadThread(peer, limit);
              void apiStore.refreshThreads();
            }}
          />
        ) : (
          <p className="border-t border-border px-4 py-3 text-xs leading-relaxed text-muted-foreground">{gate.why}</p>
        )}
      </Panel>

      <Adv>{raw ? <JsonBlock summary="raw thread (last 20 messages)" value={raw} /> : null}</Adv>
    </div>
  );
}
