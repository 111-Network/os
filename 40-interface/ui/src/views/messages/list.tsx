// The Messages list (os/messaging/README.md section 16): requests first (they need an answer), then conversations,
// newest activity first. Every row says who, the last line, how far the last own message got, and what is unread.
import { useEffect, useRef } from "react";
import { announce } from "../../app/LiveRegion";
import { useNow } from "../../app/hooks";
import { hrefTo } from "../../app/location";
import { Badge } from "../../components/badge";
import { ButtonLink } from "../../components/button";
import { Empty } from "../../components/empty";
import { SectionHeader } from "../../components/section-header";
import { useApiMeta, useIgnoredRequests, useThreads } from "../../lib/api/hooks";
import { conversationThreads, requestThreads } from "../../lib/api/messages";
import type { IgnoredRequests } from "../../lib/api/requests";
import type { Thread } from "../../lib/api/types";
import { cx, FOCUS_RING } from "../../lib/cx";
import { Adv } from "../../lib/mode";
import { MessagingBanner } from "./banner";
import { StateGlyph } from "./glyph";
import { announceIncoming, lastLine, requestItemOf, shortId, type SeenThreads, threadGlyphState, threadName, unreadText, unresolved, whenText } from "./model";
import { RequestsSection } from "./requests";

export const threadHref = (peer: string): string => hrefTo(`/messages/${peer}`);

function ThreadRow({ t, now }: { t: Thread; now: number }) {
  const glyph = threadGlyphState(t);
  const open = unresolved(t.states);
  return (
    <li className="border-b border-border last:border-b-0">
      <a href={threadHref(t.peer)} className={cx("flex min-h-11 flex-col gap-1 px-4 py-3 font-mono hover:bg-secondary/50 focus-visible:ring-inset", FOCUS_RING)}>
        <span className="flex items-baseline justify-between gap-3">
          <span className={cx("min-w-0 truncate text-sm", t.unread > 0 ? "font-bold text-foreground" : "text-foreground")}>{threadName(t)}</span>
          <span className="shrink-0 text-[11px] text-muted-foreground">{t.last ? whenText(t.last.at, now) : ""}</span>
        </span>
        <span className="flex items-center justify-between gap-3">
          <span dir="auto" className="min-w-0 truncate text-xs text-muted-foreground">
            {lastLine(t)}
          </span>
          <span className="flex shrink-0 items-center gap-2">
            {t.unread > 0 ? <Badge className="border-foreground text-foreground">{unreadText(t.unread)}</Badge> : null}
            {glyph ? <StateGlyph state={glyph} /> : null}
          </span>
        </span>
        <Adv>
          <span className="break-all text-[11px] text-muted-foreground">
            {t.label} · {shortId(t.peer)}
            {open ? ` · waiting: ${open}` : ""}
          </span>
        </Adv>
      </a>
    </li>
  );
}

/** Tell a screen reader about new unread messages and new requests, not about what was already there at load. */
function useIncomingAnnouncements(threads: Thread[] | null, ignored: IgnoredRequests): void {
  const seen = useRef<SeenThreads | null>(null);
  useEffect(() => {
    if (threads === null) return;
    const next = announceIncoming(seen.current, threads, ignored);
    if (next.say) announce(next.say);
    seen.current = next.seen;
  }, [threads, ignored]);
}

export default function ThreadList() {
  const threads = useThreads();
  const { ready } = useApiMeta();
  const now = useNow(true, 30_000);
  const ignored = useIgnoredRequests();
  useIncomingAnnouncements(threads, ignored);

  const contactsLink = (
    <ButtonLink href={hrefTo("/messages/contacts")} size="sm">
      Contacts
    </ButtonLink>
  );

  if (threads === null) {
    return (
      <div className="flex flex-col gap-6">
        <SectionHeader kicker="messages" title="Messages" level={1} end={contactsLink} />
        <MessagingBanner />
        {ready ? (
          <Empty kicker="unavailable" title="Messages could not be loaded">
            The node did not answer. Your conversations appear here as soon as it does.
          </Empty>
        ) : (
          <p role="status" className="font-mono text-xs text-muted-foreground">
            Loading messages
          </p>
        )}
      </div>
    );
  }

  const requests = requestThreads(threads).map(requestItemOf);
  const conversations = conversationThreads(threads);

  return (
    <div className="flex flex-col gap-6">
      <SectionHeader kicker="messages" title="Messages" level={1} end={contactsLink} />
      <MessagingBanner />

      <RequestsSection items={requests} headingId="requests-h" openAfterAccept />

      <section aria-labelledby="conversations-h" className="flex flex-col gap-2">
        <h2 id="conversations-h" className="font-mono text-[10px] uppercase tracking-widest text-muted-foreground">
          conversations
        </h2>
        {conversations.length === 0 ? (
          <Empty
            title="No conversations yet."
            action={
              <ButtonLink href={hrefTo("/messages/contacts")} size="sm" variant="primary">
                Add a contact
              </ButtonLink>
            }
          >
            Add someone from their card or node id, then write to them here. A message waits on this node until a path to them opens, and says so.
          </Empty>
        ) : (
          <ul className="border border-border">
            {conversations.map((t) => (
              <ThreadRow key={t.peer} t={t} now={now} />
            ))}
          </ul>
        )}
      </section>
    </div>
  );
}
