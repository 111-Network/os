import type { ReactNode } from "react";
import { cx } from "../lib/cx";

export interface TerminalChromeProps {
  /** Left part of the label: `{title} ~ {suffix}`. */
  title: string;
  /** Right part of the label. Defaults to "terminal" (DESIGN.md); the agent panel uses "chat". */
  suffix?: string;
  /** Optional slot pushed to the end of the bar (status text, small actions). */
  end?: ReactNode;
  className?: string;
}

/** The terminal header: three square dots + `{title} ~ terminal`. Decorative dots are hidden from AT. */
export function TerminalChrome({ title, suffix = "terminal", end, className }: TerminalChromeProps) {
  return (
    <div className={cx("flex items-center gap-2 border-b border-border px-4 py-2", className)}>
      <div aria-hidden="true" className="h-2 w-2 bg-foreground" />
      <div aria-hidden="true" className="h-2 w-2 bg-muted-foreground/50" />
      <div aria-hidden="true" className="h-2 w-2 bg-muted-foreground/30" />
      <span className="ms-2 min-w-0 truncate font-mono text-[10px] text-muted-foreground">
        {title} ~ {suffix}
      </span>
      {end ? <div className="ms-auto shrink-0 font-mono text-[10px] text-muted-foreground">{end}</div> : null}
    </div>
  );
}
