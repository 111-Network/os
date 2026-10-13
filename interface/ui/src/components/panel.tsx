import type { HTMLAttributes, ReactNode } from "react";
import { cx } from "../lib/cx";
import { TerminalChrome } from "./terminal-chrome";

export interface PanelProps extends Omit<HTMLAttributes<HTMLDivElement>, "title"> {
  /** When set, renders the terminal-chrome header with this title. */
  title?: string;
  /** Chrome label suffix, see TerminalChrome. */
  suffix?: string;
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
      <div className={cx(tint && "bg-secondary/20", !flush && "p-4", bodyClassName)}>{children}</div>
    </div>
  );
}
