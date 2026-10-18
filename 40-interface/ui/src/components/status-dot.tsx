import { cx } from "../lib/cx";

export type DotState = "live" | "unset" | "error" | "starting" | "unknown";

export const DOT_LABEL: Record<DotState, string> = {
  live: "LIVE",
  unset: "NOT SET",
  error: "ERROR",
  starting: "STARTING",
  unknown: "UNKNOWN",
};

/**
 * Color lives on the dot only (DESIGN.md "Status Dots"). Transitional = grey + blink, stale = hollow grey.
 * The shape/animation/label also carry the state, so color is never the only signal.
 */
const DOT: Record<DotState, string> = {
  live: "bg-status-live",
  unset: "bg-status-unset",
  error: "bg-status-error",
  starting: "bg-status-unset animate-blink",
  unknown: "border border-status-unset",
};

export interface StatusDotProps {
  state: DotState;
  /** Show the text label next to the dot. Default true. When false the label is kept for screen readers. */
  label?: boolean;
  /** Names the thing the dot describes; only used for the screen-reader text when the label is hidden. */
  name?: string;
  className?: string;
}

export function StatusDot({ state, label = true, name, className }: StatusDotProps) {
  const text = DOT_LABEL[state];
  return (
    <span className={cx("inline-flex items-center gap-2", className)}>
      <span aria-hidden="true" className={cx("inline-block h-2 w-2 shrink-0", DOT[state])} />
      {label ? (
        <span
          className={cx(
            "font-mono text-[10px] uppercase tracking-widest",
            state === "live" || state === "error" ? "text-foreground" : "text-muted-foreground",
          )}
        >
          {text}
        </span>
      ) : (
        <span className="sr-only">{name ? `${name}: ${text}` : text}</span>
      )}
    </span>
  );
}
