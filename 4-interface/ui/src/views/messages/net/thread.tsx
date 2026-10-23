// One foreign conversation (os/6-transport/README.md §3, §4, §9, §10): rows in the order they arrived here, each with the network
// chip, the native id, the sender's name as TEXT (never markup, never a link), what `auth` says about it, its delivery evidence in
// the adapter's own words, and the time it was received here. The sender's own clock sits behind a disclosure. A split send is one
// item whose state is the lowest among its parts, and cancel drops the unsent parts only.
import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState, type FormEvent } from "react";
import { navigate } from "wouter/use-hash-location";
import { redirectIfUnauthenticated } from "../../../app/AuthGate";
import { announce } from "../../../app/LiveRegion";
import { useNow } from "../../../app/hooks";
import { hrefTo } from "../../../app/location";
import { Badge } from "../../../components/badge";
import { Button, ButtonLink } from "../../../components/button";
import { Disclosure } from "../../../components/disclosure";
import { Empty } from "../../../components/empty";
import { JsonBlock } from "../../../components/json-block";
import { Panel } from "../../../components/panel";
import { SectionHeader } from "../../../components/section-header";
import { Textarea } from "../../../components/textarea";
import { apiStore, getAdapter, isApiError } from "../../../lib/api";
import { confirmGate } from "../../../lib/api/bridge";
import { useForeignThread, useNetwork } from "../../../lib/api/hooks";
import { cleanDeep, cleanPeerText } from "../../../lib/api/messages";
import {
  FOREIGN_MAX_LIMIT,
  FOREIGN_PAGE_LIMIT,
  foreignPath,
  groupParts,
  isForeignCancellable,
  isForeignRetryable,
  isHandoffPending,
  summarizeParts,
  topSeq,
  type ThreadItem,
} from "../../../lib/api/networks";
import type { ForeignMessage, Network } from "../../../lib/api/types";
import { cx } from "../../../lib/cx";
import { Adv } from "../../../lib/mode";
import { MAX_COMPOSE_CHARS, draftProblem, errorSentence, foreignComposeGate, whenText } from "../model";
import { NetworkBanner } from "./banner";
import { useConsentFlow } from "./consent";
import { ForeignGlyph, NetChip } from "./glyph";
import {
  HANDOFF_QUESTION,
  RELAYED_COPY_NOTE,
  VIA_INTERNET_TEXT,
  authText,
  capFor,
  claimLine,
  convTitle,
  evidenceText,
  foreignReasonText,
  foreignStateCopy,
  kindWord,
  nativeIdText,
  senderName,
  truncatedText,
  utf8Length,
} from "./model";

const FOLLOW_SLACK_PX = 24;
const TEXT = "font-mono text-xs leading-relaxed text-muted-foreground";

/** What a row's controls do. They are given by the thread so a fan-out result can reuse the row. */
export interface RowActions {
  retry: (ids: string[]) => void;
  cancel: (ids: string[]) => void;
  /** Open a DM with this sender's native id (a channel row offers it). Absent when the network cannot DM or the id does not fit its syntax. */
  openDm?: ((native: string) => void) | undefined;
  /** The network's own words for that action (`Network.copy.dm_note`), the same label Nearby uses. */
  dmLabel?: string | undefined;
}

function SenderLine({ m, network }: { m: ForeignMessage; network: Network | null }) {
  return (
    <div className="flex max-w-[85%] flex-wrap items-baseline gap-x-2 gap-y-0.5 text-[11px] text-muted-foreground" data-testid={`from-${m.id}`}>
      {network ? <NetChip chip={network.chip} /> : null}
      {/* The name is the stranger's own claim: text only, isolated from the page around it, wrapped wherever it must be. */}
      <span dir="auto" className="min-w-0 break-words font-mono text-sm text-foreground [unicode-bidi:isolate]" data-testid="sender-name">
        {senderName(m)}
      </span>
      <span className="break-all font-mono [unicode-bidi:isolate]" dir="ltr" data-testid="sender-native">
        {nativeIdText(m.sender.native)}
      </span>
      <span className="font-mono" data-testid="auth-label">
        {cleanPeerText(m.auth)}: {authText(network, m.auth)}
      </span>
      {m.via_internet ? <Badge>{VIA_INTERNET_TEXT}</Badge> : null}
    </div>
  );
}

function Details({ rows, now }: { rows: ForeignMessage[]; now: number }) {
  const first = rows[0]!;
  const claim = claimLine(first.claim_ts, (iso) => whenText(iso, now));
  return (
    <div className="flex flex-col gap-1 border border-border p-3 leading-relaxed">
      {first.dir === "own" ? (
        <>
          <p>{foreignStateCopy(first.state)}</p>
          {rows.map((r) => (
            <p key={r.id}>
              {r.part ? `Part ${r.part.k} of ${r.part.n}: ` : ""}
              {evidenceText(r)}
              {r.reason_code ? ` · ${foreignReasonText(r)}` : ""}
            </p>
          ))}
        </>
      ) : (
        <p>Arrived on this node. Rows are ordered by when they arrived here.</p>
      )}
      {first.relayed_from ? <p>{RELAYED_COPY_NOTE}</p> : null}
      {claim ? (
        <Disclosure summary="sender's claim" className="text-[11px]">
          <p data-testid="claim">{claim}</p>
        </Disclosure>
      ) : null}
    </div>
  );
}

function Controls({ rows, actions, label }: { rows: ForeignMessage[]; actions: RowActions; label: string }) {
  const first = rows[0]!;
  const retryable = rows.filter(isForeignRetryable);
  const cancellable = rows.filter(isForeignCancellable);
  const pending = rows.some(isHandoffPending);
  const dm = first.dir === "in" ? actions.openDm : undefined;
  if (retryable.length === 0 && cancellable.length === 0 && !dm) return null;
  return (
    <div className="flex flex-wrap items-center justify-end gap-2">
      {retryable.length > 0 ? (
        <Button size="sm" onClick={() => actions.retry(retryable.map((r) => r.id))} aria-label={`${pending ? "Resend" : "Retry sending"}: ${label}`} data-testid={`retry-${first.id}`}>
          {pending ? "resend" : first.state === "failed" ? "try again" : "retry now"}
        </Button>
      ) : null}
      {cancellable.length > 0 ? (
        <Button
          size="sm"
          confirm={rows.length > 1 ? "Drop the parts that have not left" : "Cancel sending this message"}
          onClick={() => actions.cancel(cancellable.map((r) => r.id))}
          aria-label={`Cancel sending: ${label}`}
          data-testid={`cancel-${first.id}`}
        >
          {rows.length > 1 ? `cancel unsent parts (${cancellable.length})` : "cancel sending"}
        </Button>
      ) : null}
      {dm ? (
        <Button size="sm" onClick={() => dm(first.sender.native)} data-testid={`dm-${first.id}`}>
          {actions.dmLabel ?? "Send a DM from this node (readable, not 111-sealed)"}
        </Button>
      ) : null}
    </div>
  );
}

/** One row, or the parts of one split send. */
export function ForeignRow({ item, network, now, actions }: { item: ThreadItem; network: Network | null; now: number; actions: RowActions }) {
  const rows = item.kind === "row" ? [item.m] : item.rows;
  const first = rows[0]!;
  const own = first.dir === "own";
  const parts = item.kind === "parts" ? summarizeParts(rows) : null;
  const state = parts ? parts.state : first.state;
  const reasonRow = rows.find((r) => r.reason_code) ?? first;
  const reason = own ? foreignReasonText(reasonRow) : null;
  const text = rows.map((r) => cleanPeerText(r.text)).join("\n");
  const truncated = rows.find((r) => r.truncated !== null && r.truncated !== undefined);
  const label = cleanPeerText(first.text).slice(0, 40);
  return (
    <li className={cx("flex flex-col gap-1", own ? "items-end" : "items-start")} data-testid={first.id}>
      {!own ? <SenderLine m={first} network={network} /> : null}
      <div dir="auto" className={cx("max-w-[85%] whitespace-pre-wrap break-words border px-3 py-2 text-sm leading-relaxed", own ? "border-border bg-secondary text-foreground" : "border-border text-foreground")} data-testid="row-text">
        {text}
      </div>
      {truncated ? (
        <p className={cx("max-w-[85%]", TEXT)} data-testid="truncated">
          {truncatedText(truncated.truncated ?? 0)}
        </p>
      ) : null}
      {parts ? (
        <p className={cx("max-w-[85%] text-end", TEXT)} data-testid="parts-note">
          {parts.have < parts.total ? `${parts.have} of ${parts.total} parts shown. ` : ""}
          One text cut into {parts.total} parts: it reads as {state} until every part has got further. {parts.out > 0 && parts.unsent.length > 0 ? `${parts.out} left already; cancel drops the ${parts.unsent.length} that did not.` : ""} Parts can arrive out of order.
        </p>
      ) : null}
      {reason ? (
        <p className={cx("max-w-[85%] text-end", TEXT)} data-testid="reason">
          {reason}
        </p>
      ) : null}
      {own && rows.some(isHandoffPending) ? (
        <p className="max-w-[85%] text-end font-mono text-xs font-bold text-foreground" data-testid="handoff-question">
          {HANDOFF_QUESTION}
        </p>
      ) : null}
      <Disclosure
        className="max-w-[85%] text-[11px] text-muted-foreground"
        summary={
          <span className="flex flex-wrap items-center gap-x-3 gap-y-1">
            {own ? <ForeignGlyph state={state} /> : null}
            {own ? <span data-testid="evidence">{evidenceText(parts ? { ...first, state, evidence: rows.find((r) => r.state === state)?.evidence ?? first.evidence } : first)}</span> : null}
            <span data-testid="when">
              {own ? "written" : "received"} {whenText(first.at, now)}
            </span>
            <span className="sr-only">(show details)</span>
          </span>
        }
      >
        <Details rows={rows} now={now} />
      </Disclosure>
      <Adv>
        <p className="max-w-[85%] break-all text-[11px] leading-snug text-muted-foreground" data-testid={`raw-${first.id}`}>
          {rows.map((r) => r.id).join(", ")} · seq {first.seq}
          {first.group_id ? ` · ${first.group_id}` : ""}
          {first.relayed_from ? ` · copy of ${first.relayed_from.msg} via ${first.relayed_from.edge}` : ""}
          {first.reason_code ? ` · reason ${cleanPeerText(first.reason_code)}` : ""}
        </p>
      </Adv>
      <Controls rows={rows} actions={actions} label={label} />
    </li>
  );
}

// --- The composer -------------------------------------------------------------------------------------------------------------

function Composer({ network, convId, conv, onSent }: { network: Network; convId: string; conv: NonNullable<ReturnType<typeof useForeignThread>["page"]>["conversation"]; onSent: () => void }) {
  const [value, setValue] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [oversize, setOversize] = useState<string | null>(null);
  const ref = useRef<HTMLTextAreaElement>(null);
  const { dialog, attempt } = useConsentFlow();
  const cap = capFor(network, conv);
  const bytes = utf8Length(value);

  const submit = async (e?: FormEvent, mode: "refuse" | "split" = "refuse") => {
    e?.preventDefault();
    if (busy) return;
    const problem = draftProblem(value);
    if (problem) return setError(problem);
    setBusy(true);
    setError(null);
    setOversize(null);
    try {
      const adapter = await getAdapter();
      const out = await attempt(async (hash, consent) => {
        if (hash && consent) await confirmGate(adapter, consent, hash);
        return adapter.networks.send(network.id, convId, mode === "split" ? { text: value, size_mode: "split" } : { text: value });
      });
      if (out.ok) {
        setValue("");
        onSent();
        announce("Queued on this node. The state under it says how far it got.");
      } else if (!out.cancelled) {
        const err = out.error;
        if (redirectIfUnauthenticated(err)) return;
        if (isApiError(err) && err.code === "oversize") setOversize(cleanPeerText(err.problem.detail ?? "Too long for one packet."));
        else setError(errorSentence(err, "The text could not be queued. Try again."));
      }
    } finally {
      setBusy(false);
      ref.current?.focus();
    }
  };

  return (
    <form className="flex flex-col gap-2 border-t border-border p-3" onSubmit={(e) => void submit(e)}>
      {dialog}
      <div className="flex items-end gap-2">
        <Textarea
          ref={ref}
          rows={2}
          value={value}
          maxLength={MAX_COMPOSE_CHARS}
          aria-label="Message"
          aria-describedby="foreign-composer-note"
          placeholder={`Write to ${convTitle(conv)}`}
          autoComplete="off"
          className="min-h-11 min-w-0 flex-1 resize-none! overflow-y-auto"
          onChange={(e) => {
            setValue(e.target.value);
            setError(null);
            setOversize(null);
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
      <p id="foreign-composer-note" className="text-[11px] leading-snug text-muted-foreground" data-testid="size-note">
        {bytes === 0 ? "Enter sends, Shift and Enter adds a line." : cap === null ? `${bytes} B.` : `${bytes} B of ${cap} B per packet${bytes > cap ? ": too long to send as one" : ""}.`} A copy of this text goes to {network.label}; nothing is
        exported from a 111 conversation.
      </p>
      {oversize ? (
        <div role="alert" className="flex flex-col gap-2 border border-border p-3 font-mono text-xs" data-testid="oversize">
          <p dir="auto" className="text-foreground [unicode-bidi:isolate]">Not sent: {oversize}. Nothing was queued.</p>
          <div className="flex flex-wrap gap-2">
            {network.caps.splittable ? (
              <Button size="sm" onClick={() => void submit(undefined, "split")}>
                split into numbered parts and send
              </Button>
            ) : (
              <p className="text-muted-foreground">This network cannot split a text. Shorten it.</p>
            )}
          </div>
        </div>
      ) : null}
      {error ? (
        <p role="alert" className="text-xs text-destructive">
          {error}
        </p>
      ) : null}
    </form>
  );
}

/** Retry and cancel for the rows of one conversation: the node's own sentence when it refuses, and the window re-read either way. */
export function useRowActions(netId: string, convId: string, limit: number = FOREIGN_PAGE_LIMIT): { retry: (ids: string[]) => void; cancel: (ids: string[]) => void; error: string | null; setError: (e: string | null) => void } {
  const [error, setError] = useState<string | null>(null);
  const act = useCallback(
    async (kind: "retry" | "cancel", ids: string[]) => {
      setError(null);
      try {
        const adapter = await getAdapter();
        for (const id of ids) {
          if (kind === "retry") await adapter.messages.retry(id);
          else await adapter.messages.cancel(id);
        }
        if (kind === "cancel") {
          apiStore.dropForeignRows(convId, ids);
          announce("Cancelled. What had not left is dropped.");
        }
      } catch (err) {
        if (redirectIfUnauthenticated(err)) return;
        setError(errorSentence(err, kind === "retry" ? "Could not retry that message." : "Could not cancel that message."));
      } finally {
        void apiStore.loadForeign(netId, convId, limit);
        void apiStore.refreshConversations(netId);
      }
    },
    [convId, netId, limit],
  );
  return { retry: (ids) => void act("retry", ids), cancel: (ids) => void act("cancel", ids), error, setError };
}

// --- The view ---------------------------------------------------------------------------------------------------------------------

export default function ForeignThreadView({ network: netId, conv: convId }: { network: string; conv: string }) {
  const [limit, setLimit] = useState(FOREIGN_PAGE_LIMIT);
  const slot = useForeignThread(netId, convId, limit);
  const network = useNetwork(netId);
  const now = useNow(true, 30_000);
  const page = slot.page;
  const conv = page?.conversation ?? null;
  const title = conv ? convTitle(conv) : "Conversation";

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

  // The store reads, and so marks read, a conversation a view holds open when a new text lands (a tail read never clears `unread`).
  useEffect(() => apiStore.openForeign(convId), [convId]);

  // Reading is what clears the unread count on the node; the lists follow.
  const top = page ? topSeq(page.messages) : 0;
  useEffect(() => {
    if (top > 0) void apiStore.refreshConversations(netId);
  }, [top, netId]);

  // Tell a screen reader about a new incoming text in a conversation that matters (a DM, a pinned channel), not about every channel text.
  const seenTop = useRef<number | null>(null);
  const newestIn = page ? Math.max(0, ...page.messages.filter((m) => m.dir === "in").map((m) => m.seq)) : 0;
  useEffect(() => {
    if (!conv || newestIn === 0) return;
    if (seenTop.current !== null && newestIn > seenTop.current && (conv.kind === "dm" || conv.pinned)) announce(`New message in ${title}.`);
    seenTop.current = newestIn;
  }, [newestIn, conv, title]);

  const reload = useCallback(() => void apiStore.loadForeign(netId, convId, limit), [netId, convId, limit]);
  const { retry, cancel, error: rowError, setError: setActionError } = useRowActions(netId, convId, limit);
  const actionError = rowError;

  const openDm = useMemo(() => {
    if (!network || !network.caps.kinds.includes("dm") || !network.caps.recipient_syntax) return undefined;
    let re: RegExp | null = null;
    try {
      re = new RegExp(network.caps.recipient_syntax.pattern);
    } catch {
      re = null;
    }
    if (!re || conv?.kind === "dm") return undefined;
    const pattern = re;
    return (native: string) => {
      if (!pattern.test(native)) return setActionError("That sender's id is not an address this network can send to.");
      void (async () => {
        try {
          const adapter = await getAdapter();
          const c = await adapter.networks.openDm(netId, native);
          void apiStore.refreshConversations(netId);
          navigate(foreignPath(netId, c.id));
        } catch (err) {
          if (redirectIfUnauthenticated(err)) return;
          setActionError(errorSentence(err, "Could not open that conversation."));
        }
      })();
    };
  }, [network, conv, netId]);

  const actions: RowActions = { retry, cancel, openDm, dmLabel: network?.copy.dm_note?.trim() || undefined };
  const items = useMemo(() => (page ? groupParts(page.messages) : []), [page]);
  const raw = useMemo(() => (page ? cleanDeep({ conversation: page.conversation, more: page.more, count: page.messages.length, messages: page.messages.slice(-20) }) : null), [page]);
  const gate = foreignComposeGate(network, conv);

  const back = (
    <ButtonLink href={hrefTo(foreignPath(netId))} size="sm">
      {network ? network.label : "Network"}
    </ButtonLink>
  );

  if (!page && slot.state === "loading") {
    return (
      <div className="flex flex-col gap-6">
        <SectionHeader kicker="conversation" title="Conversation" level={1} end={back} />
        <p role="status" className="font-mono text-xs text-muted-foreground">
          Loading conversation
        </p>
      </div>
    );
  }
  if (!page) {
    return (
      <div className="flex flex-col gap-6">
        <SectionHeader kicker="conversation" title="Conversation" level={1} end={back} />
        <Empty kicker={slot.state === "missing" ? "not found" : "unavailable"} title={slot.state === "missing" ? "No conversation with this id" : "The conversation could not be loaded"}>
          {slot.state === "missing" ? "This node has no such conversation." : "The node did not answer. The conversation appears here as soon as it does."}
        </Empty>
      </div>
    );
  }

  return (
    <div className="flex flex-col gap-6">
      <SectionHeader kicker={`${network ? network.label : netId} · ${kindWord(page.conversation.kind)}`} title={title} level={1} end={back} />
      {network ? <NetworkBanner network={network} /> : null}
      <Adv>
        <p className="break-all font-mono text-[11px] text-muted-foreground" data-testid="conv-native">
          {convId} · {nativeIdText(page.conversation.native_id, 80)}
        </p>
      </Adv>

      <Panel title="conversation" suffix={title} flush className="flex flex-col" bodyClassName="flex min-h-0 flex-1 flex-col font-mono">
        <div
          ref={listRef}
          role="log"
          aria-live="off"
          aria-label={`Conversation ${title}`}
          tabIndex={0}
          onScroll={onScroll}
          className="flex max-h-[60dvh] min-h-48 flex-col gap-4 overflow-y-auto px-4 py-4 focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-foreground focus-visible:outline-none"
        >
          {page.more && limit < FOREIGN_MAX_LIMIT ? (
            <div className="flex justify-center">
              <Button size="sm" onClick={() => setLimit((l) => Math.min(l + FOREIGN_PAGE_LIMIT, FOREIGN_MAX_LIMIT))}>
                show earlier messages
              </Button>
            </div>
          ) : null}
          {items.length === 0 ? (
            <p className="m-auto max-w-prose text-center text-xs leading-relaxed text-muted-foreground">Nothing here yet.</p>
          ) : (
            <ul className="flex flex-col gap-4">
              {items.map((it) => (
                <ForeignRow key={it.kind === "row" ? it.m.id : it.key} item={it} network={network} now={now} actions={actions} />
              ))}
            </ul>
          )}
        </div>
        {actionError ? (
          <p role="alert" dir="auto" className="border-t border-border px-4 py-2 text-xs text-destructive [unicode-bidi:isolate]" data-testid="action-error">
            {actionError}
          </p>
        ) : null}
        {slot.state === "error" ? (
          <p role="status" className="border-t border-border px-4 py-2 text-xs text-muted-foreground">
            The latest messages could not be loaded.{" "}
            <button type="button" className="underline underline-offset-4" onClick={reload}>
              try again
            </button>
          </p>
        ) : null}
        {gate.ok && network ? (
          <>
            <ul className="flex flex-col gap-1 border-t border-border px-4 py-3" aria-label="Read before you send" data-testid="gate-notes">
              {gate.notes.map((n) => (
                <li key={n.code} className={cx(TEXT, n.tone === "warn" && "text-foreground")} data-testid={`gate-${n.code}`}>
                  {n.tone === "warn" ? "Note: " : ""}
                  {n.text}
                  {n.href ? (
                    <>
                      {" "}
                      <a href={hrefTo(n.href)} className="underline underline-offset-4">
                        Open settings
                      </a>
                    </>
                  ) : null}
                </li>
              ))}
            </ul>
            <Composer
              network={network}
              convId={convId}
              conv={page.conversation}
              onSent={() => {
                pinned.current = true;
                reload();
                void apiStore.refreshConversations(netId);
              }}
            />
          </>
        ) : (
          <p className="border-t border-border px-4 py-3 text-xs leading-relaxed text-muted-foreground" data-testid="gate-why">
            {gate.ok ? "Loading this network." : gate.why}
          </p>
        )}
      </Panel>

      <Adv>{raw ? <JsonBlock summary="raw conversation (last 20 rows)" value={raw} /> : null}</Adv>
    </div>
  );
}
