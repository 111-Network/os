// A network's conversations split into channels, DMs and recipient lists (os/6-transport/README.md §10 "each split into channels and
// DMs"). Titles are the air's own words: text only. A DM's unread count shows; a channel's only when it is pinned (the attention rule).
import { hrefTo } from "../../../app/location";
import { Badge } from "../../../components/badge";
import { countsAsUnread } from "../../../lib/attention";
import { channelsOf, dmsOf, foreignPath, groupsOf } from "../../../lib/api/networks";
import type { ForeignConversation } from "../../../lib/api/types";
import { cx, FOCUS_RING } from "../../../lib/cx";
import { convTitle } from "./model";

export function ConversationRow({ c }: { c: ForeignConversation }) {
  const unread = countsAsUnread(c) && c.unread > 0;
  return (
    <li className="border-b border-border last:border-b-0">
      <a href={hrefTo(foreignPath(c.network, c.id))} className={cx("flex min-h-11 items-center justify-between gap-3 px-4 py-2 font-mono hover:bg-secondary/50 focus-visible:ring-inset", FOCUS_RING)} data-testid={`conv-${c.id}`}>
        <span dir="auto" className={cx("min-w-0 truncate text-sm [unicode-bidi:isolate]", unread ? "font-bold text-foreground" : "text-foreground")}>
          {convTitle(c)}
        </span>
        <span className="flex shrink-0 items-center gap-2">
          {c.pinned ? <Badge>pinned</Badge> : null}
          {c.uplink === true ? <Badge className="border-foreground text-foreground">uplinked</Badge> : null}
          {unread ? <Badge className="border-foreground text-foreground">{c.unread} unread</Badge> : null}
        </span>
      </a>
    </li>
  );
}

function Group({ id, title, rows }: { id: string; title: string; rows: ForeignConversation[] }) {
  if (rows.length === 0) return null;
  return (
    <section aria-labelledby={id} className="flex flex-col gap-1">
      <h3 id={id} className="font-mono text-[10px] uppercase tracking-widest text-muted-foreground">
        {title}
      </h3>
      <ul className="border border-border">
        {rows.map((c) => (
          <ConversationRow key={c.id} c={c} />
        ))}
      </ul>
    </section>
  );
}

/** Channels, then DMs, then recipient lists; a kind with nothing in it shows nothing. */
export function SplitConversations({ convs, idPrefix }: { convs: readonly ForeignConversation[]; idPrefix: string }) {
  return (
    <div className="flex flex-col gap-3">
      <Group id={`${idPrefix}-ch`} title="channels" rows={channelsOf(convs)} />
      <Group id={`${idPrefix}-dm`} title="DMs" rows={dmsOf(convs)} />
      <Group id={`${idPrefix}-gr`} title="recipient lists" rows={groupsOf(convs)} />
    </div>
  );
}
