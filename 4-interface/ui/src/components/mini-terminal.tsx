import { useEffect, useRef } from "react";
import { cx, FOCUS_RING } from "../lib/cx";
import { TerminalChrome } from "./terminal-chrome";

export interface MiniTerminalProps {
  lines: string[];
  title: string;
  /** Chrome suffix, default "terminal". */
  suffix?: string;
  /** Keep the view pinned to the newest line. Pauses while the reader has scrolled up, resumes at the bottom. */
  follow?: boolean;
  /** Show the trailing blinking block cursor (the tail is being fed). */
  live?: boolean;
  /** Tailwind max-height class for the scroll area. */
  maxHeightClass?: string;
  /** Shown when `lines` is empty. */
  emptyText?: string;
  className?: string;
}

/**
 * Log tail. Renders all lines at once: there is deliberately no typewriter, because live data
 * must never be delayed or animated in.
 */
export function MiniTerminal({
  lines,
  title,
  suffix,
  follow = false,
  live = false,
  maxHeightClass = "max-h-64",
  emptyText,
  className,
}: MiniTerminalProps) {
  const bodyRef = useRef<HTMLDivElement>(null);
  const pinned = useRef(true);

  const onScroll = () => {
    const el = bodyRef.current;
    if (el) pinned.current = el.scrollHeight - el.scrollTop - el.clientHeight < 24;
  };

  useEffect(() => {
    if (follow) pinned.current = true;
  }, [follow]);

  useEffect(() => {
    const el = bodyRef.current;
    if (follow && el && pinned.current) el.scrollTop = el.scrollHeight;
  }, [lines, follow, live]);

  return (
    <div className={cx("flex flex-col border border-border", className)}>
      <TerminalChrome title={title} suffix={suffix} />
      {/* role=log with aria-live=off: a labelled, keyboard-scrollable region that does not chatter at screen readers. */}
      <div
        ref={bodyRef}
        role="log"
        aria-live="off"
        aria-label={`${title} output`}
        tabIndex={0}
        onScroll={onScroll}
        className={cx(
          "flex-1 overflow-y-auto bg-secondary/20 p-4 font-mono text-xs leading-relaxed focus-visible:ring-inset",
          maxHeightClass,
          FOCUS_RING,
        )}
      >
        <div className="flex flex-col gap-0.5">
          {lines.length === 0 && emptyText ? <div className="text-muted-foreground">{emptyText}</div> : null}
          {lines.map((line, i) => (
            <div
              key={i}
              className={cx("whitespace-pre-wrap break-words", line.startsWith("$") ? "text-foreground" : "text-muted-foreground")}
            >
              {line === "" ? " " : line}
            </div>
          ))}
          {live ? (
            <div aria-hidden="true" className="text-foreground">
              <span className="animate-blink">{"█"}</span>
            </div>
          ) : null}
        </div>
      </div>
    </div>
  );
}
