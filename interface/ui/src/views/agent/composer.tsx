// The input row: `>` glyph, an auto-growing textarea (1-6 rows), and send / stop.
import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { Button } from "../../components/button";
import { Textarea } from "../../components/textarea";
import { MAX_MESSAGE_CHARS } from "./state";
import type { ChatSession } from "./session";

const MAX_ROWS = 6;

/** Grow the box to its content, between one row and MAX_ROWS rows. */
function fit(el: HTMLTextAreaElement): void {
  const cs = getComputedStyle(el);
  const fontSize = parseFloat(cs.fontSize) || 14;
  const line = parseFloat(cs.lineHeight) || fontSize * 1.625;
  const chrome = (parseFloat(cs.paddingTop) || 0) + (parseFloat(cs.paddingBottom) || 0) + (parseFloat(cs.borderTopWidth) || 0) + (parseFloat(cs.borderBottomWidth) || 0);
  el.style.height = "auto";
  el.style.height = `${Math.min(el.scrollHeight + (parseFloat(cs.borderTopWidth) || 0) + (parseFloat(cs.borderBottomWidth) || 0), line * MAX_ROWS + chrome)}px`;
}

export interface ComposerProps {
  session: ChatSession;
  streaming: boolean;
  /** Changes whenever the session puts new text into the box (staged prompt, restored message). */
  draftSeq: number;
  /** A message was accepted (sent or queued): the list should follow the bottom again. */
  onSubmitted: () => void;
}

export function Composer({ session, streaming, draftSeq, onSubmitted }: ComposerProps) {
  const ref = useRef<HTMLTextAreaElement>(null);
  const [value, setValue] = useState(() => session.draft);
  const seen = useRef(draftSeq);

  // Text handed over by the session: show it, focus the box, caret at the end. Never sent from here.
  useEffect(() => {
    if (seen.current === draftSeq) return;
    seen.current = draftSeq;
    setValue(session.draft);
    const el = ref.current;
    if (el) {
      el.focus();
      requestAnimationFrame(() => el.setSelectionRange(el.value.length, el.value.length));
    }
  }, [draftSeq, session]);

  useLayoutEffect(() => {
    if (ref.current) fit(ref.current);
  }, [value]);

  const change = (next: string) => {
    setValue(next);
    session.typeDraft(next);
  };

  const submit = () => {
    if (streaming) return;
    if (session.send(value) === "rejected") return;
    change("");
    onSubmitted();
  };

  const empty = value.trim() === "";
  return (
    <form
      className="flex items-end gap-2 border-t border-border p-3"
      onSubmit={(e) => {
        e.preventDefault();
        submit();
      }}
    >
      <span aria-hidden="true" className="flex min-h-11 select-none items-center font-mono text-sm text-muted-foreground">
        {">"}
      </span>
      <Textarea
        ref={ref}
        rows={1}
        value={value}
        maxLength={MAX_MESSAGE_CHARS}
        aria-label="Message to the agent"
        aria-describedby="composer-hint"
        placeholder="Message the agent"
        autoComplete="off"
        className="min-h-11 min-w-0 flex-1 resize-none! overflow-y-auto"
        onChange={(e) => change(e.target.value)}
        onKeyDown={(e) => {
          if (e.key !== "Enter" || e.shiftKey || e.nativeEvent.isComposing) return;
          e.preventDefault();
          submit();
        }}
      />
      {/* One button, two jobs: a swapped element would lose the keyboard user's focus. */}
      <Button
        type={streaming ? "button" : "submit"}
        size="sm"
        variant={streaming ? "default" : "primary"}
        aria-disabled={!streaming && empty ? true : undefined}
        onClick={
          streaming
            ? () => {
                session.stop();
                ref.current?.focus();
              }
            : undefined
        }
      >
        {streaming ? "stop" : "send"}
      </Button>
      <span id="composer-hint" className="sr-only">
        Enter sends. Shift and Enter adds a new line.
      </span>
    </form>
  );
}
