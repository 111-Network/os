import type { HTMLAttributes, ReactNode } from "react";
import { cx } from "../lib/cx";
import { TerminalChrome } from "./terminal-chrome";

export interface PanelProps extends Omit<HTMLAttributes<HTMLDivElement>, "title"> {
  /** When set, renders the terminal-chrome header with this title. */
  title?: string;
  /** Chrome label suffix, see TerminalChrome. */
  suffix?: string;
  /**
   * Also expose the title as a visually hidden heading of this level, so the panel is a section in the
   * page outline (the chrome bar itself is not a heading).
   */
  heading?: 2 | 3;
  /** Slot at the end of the chrome bar. Ignored without a title. */
  chromeEnd?: ReactNode;
  /** Terminal-body tint (`bg-secondary/20`). */
  tint?: boolean;
  /** The one layered shadow (`--shadow-panel`). */
  elevated?: boolean;
  /** Drop the default body padding (tables and terminals bring their own). */
  flush?: boolean;
  bodyClassName?: string;
  children?: ReactNode;
}

export function Panel({
  title,
  suffix,
  heading,
  chromeEnd,
  tint = false,
  elevated = false,
  flush = false,
  className,
  bodyClassName,
  style,
  children,
  ...rest
}: PanelProps) {
  return (
    <div
      {...rest}
      className={cx("border border-border", className)}
      style={elevated ? { boxShadow: "var(--shadow-panel)", ...style } : style}
    >
      {title !== undefined ? <TerminalChrome title={title} suffix={suffix} end={chromeEnd} /> : null}
      <div className={cx(tint && "bg-secondary/20", !flush && "p-4", bodyClassName)}>
        {heading && title ? (heading === 2 ? <h2 className="sr-only">{title}</h2> : <h3 className="sr-only">{title}</h3>) : null}
        {children}
      </div>
    </div>
  );
}
