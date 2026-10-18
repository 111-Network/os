// Transcript rows, the live (streaming) bubble, queued messages and the empty-state greeting.
import { memo } from "react";
import { Badge } from "../../components/badge";
import { Button } from "../../components/button";
import type { ChatRow, Starter } from "./state";
import type { LiveTurn, QueuedMessage } from "./session";

function ToolLines({ tools }: { tools?: string[] | undefined }) {
  if (!tools || tools.length === 0) return null;
  return (
    <ul className="mt-1 flex flex-col gap-0.5 font-mono text-xs text-muted-foreground">
      {tools.map((t, i) => (
        <li key={i}>{t}</li>
      ))}
    </ul>
  );
}

function UserText({ text }: { text: string }) {
  return (
    <p className="min-w-0 flex-1 whitespace-pre-wrap break-words text-muted-foreground">
      <span className="sr-only">You: </span>
      {text}
    </p>
  );
}

function Prompt() {
  return (
    <span aria-hidden="true" className="select-none text-muted-foreground">
      {">"}
    </span>
  );
}

const Row = memo(function Row({ row }: { row: ChatRow }) {
  if (row.role === "user") {
    return (
      <div className="flex gap-2">
        <Prompt />
        <UserText text={row.text} />
      </div>
    );
  }
  return (
    <div className="border-s border-border ps-3">
      {row.text !== "" ? (
        <p className="whitespace-pre-wrap break-words text-foreground">
          <span className="sr-only">Agent: </span>
          {row.text}
        </p>
      ) : null}
      <ToolLines tools={row.tools} />
      {row.stopped ? <p className="mt-1 text-xs text-muted-foreground">[stopped]</p> : null}
      {row.error ? (
        <p className="mt-1 whitespace-pre-wrap break-words text-destructive">
          <span className="sr-only">Error: </span>
          {row.error}
        </p>
      ) : null}
    </div>
  );
});

/** The reply being streamed: text so far, a block cursor, and the tool lines it has used. */
export function LiveBubble({ live }: { live: LiveTurn }) {
  return (
    <div className="border-s border-foreground ps-3" data-testid="live-bubble">
      <p className="whitespace-pre-wrap break-words text-foreground">
        <span className="sr-only">Agent is replying: </span>
        {live.text}
        <span aria-hidden="true" className="animate-blink">
          {"█"}
        </span>
      </p>
      <ToolLines tools={live.tools} />
    </div>
  );
}

export interface QueuedListProps {
  queued: QueuedMessage[];
  onRemove: (id: string) => void;
  onRetry: () => void;
}

export function QueuedList({ queued, onRemove, onRetry }: QueuedListProps) {
  if (queued.length === 0) return null;
  return (
    <ul aria-label="Queued messages" className="flex flex-col gap-3">
      {queued.map((q, i) => (
        <li key={q.id} className="flex flex-wrap items-start gap-2">
          <Prompt />
          <p className="min-w-0 flex-1 whitespace-pre-wrap break-words text-muted-foreground">
            <span className="sr-only">You, queued: </span>
            {q.text}
          </p>
          <span className="flex shrink-0 items-center gap-2">
            <Badge>QUEUED</Badge>
            {i === 0 ? (
              <Button size="sm" onClick={onRetry} aria-label="Try sending the queued messages now">
                try now
              </Button>
            ) : null}
            <Button size="sm" onClick={() => onRemove(q.id)} aria-label="Remove this queued message">
              remove
            </Button>
          </span>
        </li>
      ))}
    </ul>
  );
}

export interface MessageListProps {
  rows: ChatRow[];
  live: LiveTurn | null;
}

export function Transcript({ rows, live }: MessageListProps) {
  return (
    <>
      {rows.map((r) => (
        <Row key={r.key} row={r} />
      ))}
      {live ? <LiveBubble live={live} /> : null}
    </>
  );
}

/** First-message area: not a chat message, just a nudge with prepared starting points. */
export function Greeting({ starters, onStage }: { starters: Starter[]; onStage: (text: string) => void }) {
  return (
    <div className="border border-dashed border-border p-4">
      <p className="max-w-prose text-muted-foreground">
        {`Ask anything about this node. Try: ${starters.map((s) => `'${s.label}'`).join(", ")}`}
      </p>
      <div className="mt-3 flex flex-wrap gap-2">
        {starters.map((s) => (
          <Button key={s.label} size="sm" onClick={() => onStage(s.text)}>
            {s.label}
          </Button>
        ))}
      </div>
      <p className="mt-3 text-[10px] uppercase tracking-widest text-muted-foreground">
        A button fills the message box. Nothing is sent until you send it.
      </p>
    </div>
  );
}
